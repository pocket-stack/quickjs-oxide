use std::error::Error as StdError;
use std::fmt;

/// A byte and human-readable location in a UTF-8 source file.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceLocation {
    pub byte_offset: usize,
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    #[must_use]
    pub const fn new(byte_offset: usize, line: u32, column: u32) -> Self {
        Self {
            byte_offset,
            line,
            column,
        }
    }
}

/// Half-open source range `[start, end)`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl SourceSpan {
    #[must_use]
    pub const fn new(start: SourceLocation, end: SourceLocation) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Syntax,
    Type,
    Reference,
    Range,
    /// A JavaScript-visible QuickJS `InternalError`, distinct from an engine
    /// invariant or implementation fault.
    JsInternal,
    Internal,
    Io,
}

/// QuickJS native Error subclasses, in the same stable order as
/// `JSErrorEnum` in the 2026-06-04 baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum NativeErrorKind {
    Eval,
    Range,
    Reference,
    Syntax,
    Type,
    Uri,
    Internal,
    Aggregate,
}

/// Raw bytes produced by QuickJS's stack-local `char buf[256]` native-error
/// formatter. The payload silently stops at 255 bytes; JavaScript String
/// construction observes only the prefix before the first formatted NUL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeErrorMessage {
    bytes: [u8; 256],
    len: usize,
}

impl NativeErrorMessage {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
        }
    }

    #[must_use]
    pub(crate) fn from_utf8(value: &str) -> Self {
        let mut message = Self::new();
        message.push_bytes(value.bytes());
        message
    }

    pub(crate) fn push_bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            if self.len == self.bytes.len() - 1 {
                break;
            }
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }

    pub(crate) fn push_c_string_bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            if byte == 0 || self.len == self.bytes.len() - 1 {
                break;
            }
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }

    pub(crate) fn push_utf8(&mut self, value: &str) {
        self.push_bytes(value.bytes());
    }

    #[must_use]
    pub(crate) fn visible_bytes(&self) -> &[u8] {
        let visible_len = self.bytes[..self.len]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.len);
        &self.bytes[..visible_len]
    }
}

impl NativeErrorKind {
    pub const ALL: [Self; 8] = [
        Self::Eval,
        Self::Range,
        Self::Reference,
        Self::Syntax,
        Self::Type,
        Self::Uri,
        Self::Internal,
        Self::Aggregate,
    ];
    pub const COUNT: usize = Self::ALL.len();

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Eval => "EvalError",
            Self::Range => "RangeError",
            Self::Reference => "ReferenceError",
            Self::Syntax => "SyntaxError",
            Self::Type => "TypeError",
            Self::Uri => "URIError",
            Self::Internal => "InternalError",
            Self::Aggregate => "AggregateError",
        }
    }

    /// Classify an engine-facing error kind which has JavaScript throw
    /// semantics. `Internal` and `Io` are deliberately excluded: current
    /// internal errors include malformed bytecode and implementation gaps,
    /// neither of which may become catchable JavaScript exceptions.
    #[must_use]
    pub const fn from_javascript_error(kind: ErrorKind) -> Option<Self> {
        match kind {
            ErrorKind::Syntax => Some(Self::Syntax),
            ErrorKind::Type => Some(Self::Type),
            ErrorKind::Reference => Some(Self::Reference),
            ErrorKind::Range => Some(Self::Range),
            ErrorKind::JsInternal => Some(Self::Internal),
            ErrorKind::Internal | ErrorKind::Io => None,
        }
    }
}

/// An engine error. JavaScript exceptions will eventually carry a heap value;
/// this type is also usable before a context exists (lexer and decoder errors).
#[derive(Clone)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    native_message: Option<Box<NativeErrorMessage>>,
    span: Option<SourceSpan>,
}

impl Error {
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind,
            message,
            native_message: None,
            span: None,
        }
    }

    #[must_use]
    pub(crate) fn from_native_message(kind: ErrorKind, native_message: NativeErrorMessage) -> Self {
        let message = native_message.to_utf8_lossy();
        Self {
            kind,
            message,
            native_message: Some(Box::new(native_message)),
            span: None,
        }
    }

    #[must_use]
    pub fn syntax(message: impl Into<String>, span: SourceSpan) -> Self {
        Self::new(ErrorKind::Syntax, message).with_span(span)
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub(crate) fn native_message(&self) -> Option<&NativeErrorMessage> {
        self.native_message.as_deref()
    }

    #[must_use]
    pub const fn span(&self) -> Option<SourceSpan> {
        self.span
    }

    #[must_use]
    pub const fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("span", &self.span)
            .finish()
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.message == other.message && self.span == other.span
    }
}

impl Eq for Error {}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            if self.kind == ErrorKind::JsInternal {
                write!(
                    formatter,
                    "InternalError at {}:{}: {}",
                    span.start.line, span.start.column, self.message
                )
            } else {
                write!(
                    formatter,
                    "{:?}Error at {}:{}: {}",
                    self.kind, span.start.line, span.start.column, self.message
                )
            }
        } else {
            match self.kind {
                ErrorKind::JsInternal => write!(formatter, "InternalError: {}", self.message),
                _ => write!(formatter, "{:?}Error: {}", self.kind, self.message),
            }
        }
    }
}

impl StdError for Error {}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind, NativeErrorMessage};

    #[test]
    fn exact_native_sidecar_preserves_public_error_compatibility() {
        let mut raw = NativeErrorMessage::new();
        raw.push_bytes([0x80, b'A']);
        let exact = Error::from_native_message(ErrorKind::Type, raw);
        let public = Error::new(ErrorKind::Type, "\u{fffd}");

        assert_eq!(exact.message(), "\u{fffd}");
        assert_eq!(exact, public);
        assert_eq!(format!("{exact:?}"), format!("{public:?}"));
        assert_eq!(
            format!("{exact:?}"),
            "Error { kind: Type, message: \"�\", span: None }"
        );
        assert_eq!(format!("{exact}"), "TypeError: �");
        assert_eq!(exact.clone(), exact);
        assert_eq!(
            exact.native_message().unwrap().visible_bytes(),
            [0x80, b'A']
        );
        assert!(public.native_message().is_none());
    }
}
