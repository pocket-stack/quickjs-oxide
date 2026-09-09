use crate::engine::value::{JsString, JsStringBuilder, JsStringError};

pub(crate) struct DynamicSourceBuilder {
    pub(crate) source: JsStringBuilder,
}

impl DynamicSourceBuilder {
    pub(crate) fn new() -> Self {
        Self::with_limit(JsString::MAX_LEN)
    }

    pub(crate) fn with_limit(limit: usize) -> Self {
        let limit = limit.min(JsString::MAX_LEN);
        Self {
            source: JsStringBuilder::with_limit(64.min(limit), limit),
        }
    }

    pub(crate) fn push_str(&mut self, value: &str) -> Result<(), JsStringError> {
        self.source.push_utf8(value)
    }

    pub(crate) fn push_js_string(&mut self, value: &JsString) -> Result<(), JsStringError> {
        self.source.push_js_string(value)
    }

    pub(crate) fn finish(self) -> Result<JsString, JsStringError> {
        self.source.finish()
    }
}
