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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    span: Option<SourceSpan>,
}

impl Error {
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
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
    pub const fn span(&self) -> Option<SourceSpan> {
        self.span
    }

    #[must_use]
    pub const fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }
}

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
