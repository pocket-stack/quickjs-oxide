//! ECMAScript lexical analysis.
//!
//! This module is intentionally independent from the rest of the engine and
//! from third-party crates. The interface preserves two parser/lexer
//! boundaries which matter for ECMAScript:
//!
//! * A slash is division under LexicalGoal::Div and starts a regular
//!   expression under LexicalGoal::RegExp. A parser, rather than a lexer
//!   heuristic, knows which interpretation is valid.
//! * After the parser consumes the right brace ending a template
//!   substitution, it asks for LexicalGoal::TemplateContinuation.
//!
//! This is the same broad division of responsibility used by QuickJS. It
//! prevents valid division, regular-expression, and template source from being
//! silently assigned the wrong meaning before a parser exists.
//!
//! Staged limitation: Unicode ID_Start and ID_Continue tables are not present
//! yet. Non-ASCII identifier input therefore returns
//! LexErrorKind::UnsupportedUnicodeIdentifier instead of being split,
//! replaced, or accepted with incomplete rules.

use std::fmt;
use std::iter::FusedIterator;

const TEMPLATE_QUOTE: char = '\u{0060}';

/// A location in the original UTF-8 source.
///
/// byte_offset is zero-based. line and column are one-based. Columns count
/// Unicode scalar values, not bytes; CRLF counts as one line terminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub byte_offset: usize,
    pub line: u32,
    pub column: u32,
}

impl Position {
    pub const fn new(byte_offset: usize, line: u32, column: u32) -> Self {
        Self {
            byte_offset,
            line,
            column,
        }
    }
}

/// A half-open source range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub const fn is_empty(self) -> bool {
        self.start.byte_offset == self.end.byte_offset
    }
}

/// Context which changes whether QuickJS treats a word as a keyword.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LexContext {
    pub strict: bool,
    pub module: bool,
    pub generator: bool,
    pub async_function: bool,
}

/// Lexer construction options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LexerOptions {
    pub context: LexContext,
    /// Enables Annex B line comments beginning with the HTML open/close forms.
    pub allow_html_comments: bool,
}

/// The lexical grammar requested by the parser for the next token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LexicalGoal {
    /// A slash is a division punctuator. This is the iterator default.
    #[default]
    Div,
    /// A slash begins a regular-expression literal.
    RegExp,
    /// Resume a template immediately after a substitution's right brace.
    TemplateContinuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    Null,
    False,
    True,
    If,
    Else,
    Return,
    Var,
    This,
    Delete,
    Void,
    Typeof,
    New,
    In,
    Instanceof,
    Do,
    While,
    For,
    Break,
    Continue,
    Switch,
    Case,
    Default,
    Throw,
    Try,
    Catch,
    Finally,
    Function,
    Debugger,
    With,
    Class,
    Const,
    Enum,
    Export,
    Extends,
    Import,
    Super,
    Implements,
    Interface,
    Let,
    Package,
    Private,
    Protected,
    Public,
    Static,
    Yield,
    Await,
}

impl Keyword {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::False => "false",
            Self::True => "true",
            Self::If => "if",
            Self::Else => "else",
            Self::Return => "return",
            Self::Var => "var",
            Self::This => "this",
            Self::Delete => "delete",
            Self::Void => "void",
            Self::Typeof => "typeof",
            Self::New => "new",
            Self::In => "in",
            Self::Instanceof => "instanceof",
            Self::Do => "do",
            Self::While => "while",
            Self::For => "for",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::Switch => "switch",
            Self::Case => "case",
            Self::Default => "default",
            Self::Throw => "throw",
            Self::Try => "try",
            Self::Catch => "catch",
            Self::Finally => "finally",
            Self::Function => "function",
            Self::Debugger => "debugger",
            Self::With => "with",
            Self::Class => "class",
            Self::Const => "const",
            Self::Enum => "enum",
            Self::Export => "export",
            Self::Extends => "extends",
            Self::Import => "import",
            Self::Super => "super",
            Self::Implements => "implements",
            Self::Interface => "interface",
            Self::Let => "let",
            Self::Package => "package",
            Self::Private => "private",
            Self::Protected => "protected",
            Self::Public => "public",
            Self::Static => "static",
            Self::Yield => "yield",
            Self::Await => "await",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Punctuator {
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Dot,
    Ellipsis,
    Semicolon,
    Comma,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    EqualEqual,
    StrictEqual,
    Not,
    NotEqual,
    StrictNotEqual,
    Plus,
    Minus,
    Multiply,
    Divide,
    Remainder,
    Exponent,
    Increment,
    Decrement,
    ShiftLeft,
    ShiftRight,
    UnsignedShiftRight,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    LogicalAnd,
    LogicalOr,
    NullishCoalesce,
    Question,
    OptionalChain,
    Colon,
    Arrow,
    PlusAssign,
    MinusAssign,
    MultiplyAssign,
    DivideAssign,
    RemainderAssign,
    ExponentAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    UnsignedShiftRightAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    LogicalAndAssign,
    LogicalOrAssign,
    NullishAssign,
}

impl Punctuator {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeftBrace => "{",
            Self::RightBrace => "}",
            Self::LeftParen => "(",
            Self::RightParen => ")",
            Self::LeftBracket => "[",
            Self::RightBracket => "]",
            Self::Dot => ".",
            Self::Ellipsis => "...",
            Self::Semicolon => ";",
            Self::Comma => ",",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::Equal => "=",
            Self::EqualEqual => "==",
            Self::StrictEqual => "===",
            Self::Not => "!",
            Self::NotEqual => "!=",
            Self::StrictNotEqual => "!==",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Remainder => "%",
            Self::Exponent => "**",
            Self::Increment => "++",
            Self::Decrement => "--",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
            Self::UnsignedShiftRight => ">>>",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::BitNot => "~",
            Self::LogicalAnd => "&&",
            Self::LogicalOr => "||",
            Self::NullishCoalesce => "??",
            Self::Question => "?",
            Self::OptionalChain => "?.",
            Self::Colon => ":",
            Self::Arrow => "=>",
            Self::PlusAssign => "+=",
            Self::MinusAssign => "-=",
            Self::MultiplyAssign => "*=",
            Self::DivideAssign => "/=",
            Self::RemainderAssign => "%=",
            Self::ExponentAssign => "**=",
            Self::ShiftLeftAssign => "<<=",
            Self::ShiftRightAssign => ">>=",
            Self::UnsignedShiftRightAssign => ">>>=",
            Self::BitAndAssign => "&=",
            Self::BitOrAssign => "|=",
            Self::BitXorAssign => "^=",
            Self::LogicalAndAssign => "&&=",
            Self::LogicalOrAssign => "||=",
            Self::NullishAssign => "??=",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericRadix {
    Binary,
    Octal,
    Decimal,
    Hexadecimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberKind {
    Integer(NumericRadix),
    Float,
    BigInt(NumericRadix),
    /// Annex B leading-zero integer syntax accepted outside strict mode.
    LegacyOctal,
    /// Annex B leading-zero syntax containing 8 or 9, evaluated as decimal.
    LegacyDecimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberLiteral<'a> {
    pub raw: &'a str,
    pub kind: NumberKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quote {
    Single,
    Double,
}

/// JavaScript string contents in their native UTF-16 code-unit form.
///
/// Rust String cannot represent lone surrogates, but ECMAScript string escape
/// sequences can. Keeping UTF-16 here avoids losing that language distinction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct JsString {
    pub utf16: Vec<u16>,
}

impl JsString {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_utf8(value: &str) -> Self {
        Self {
            utf16: value.encode_utf16().collect(),
        }
    }

    pub fn push_char(&mut self, ch: char) {
        let mut units = [0_u16; 2];
        self.utf16.extend_from_slice(ch.encode_utf16(&mut units));
    }

    pub fn push_code_point(&mut self, value: u32) {
        if value <= 0xffff {
            self.utf16.push(value as u16);
        } else {
            let adjusted = value - 0x1_0000;
            self.utf16.push(0xd800 | ((adjusted >> 10) as u16));
            self.utf16.push(0xdc00 | ((adjusted & 0x3ff) as u16));
        }
    }

    pub fn to_string(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.utf16)
    }

    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.utf16)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringLiteral<'a> {
    pub raw: &'a str,
    pub value: JsString,
    pub quote: Quote,
    pub has_escape: bool,
    pub has_legacy_octal_escape: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplatePartKind {
    NoSubstitution,
    Head,
    Middle,
    Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateEscapeError {
    pub message: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplatePart<'a> {
    /// Source between delimiters, before escape processing.
    pub raw: &'a str,
    /// ECMAScript template raw value, including backslashes and with CR or
    /// CRLF normalized to LF.
    pub raw_value: JsString,
    /// None is intentional: tagged templates observe undefined cooked text
    /// for a malformed escape, while untagged templates must reject it.
    pub cooked: Option<JsString>,
    pub invalid_escape: Option<TemplateEscapeError>,
    pub kind: TemplatePartKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegExpLiteral<'a> {
    pub raw: &'a str,
    pub pattern: &'a str,
    pub flags: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifier<'a> {
    pub raw: &'a str,
    pub value: String,
    pub has_escape: bool,
    /// Set even when parser context means the word remains an identifier.
    pub keyword_hint: Option<Keyword>,
    /// QuickJS keeps escaped reserved words as identifiers so the parser can
    /// issue the context-appropriate reserved-identifier diagnostic.
    pub escaped_reserved_word: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind<'a> {
    Identifier(Identifier<'a>),
    PrivateIdentifier(Identifier<'a>),
    Keyword(Keyword),
    Number(NumberLiteral<'a>),
    String(StringLiteral<'a>),
    Template(TemplatePart<'a>),
    RegExp(RegExpLiteral<'a>),
    Punctuator(Punctuator),
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub span: Span,
    /// True when trivia before this token contained at least one ECMAScript
    /// line terminator. Parsers use this for ASI and restricted productions.
    pub line_terminator_before: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LexErrorKind {
    UnexpectedCharacter,
    UnterminatedComment,
    UnterminatedString,
    UnterminatedTemplate,
    InvalidEscape,
    InvalidNumber,
    UnsupportedUnicodeIdentifier,
    InvalidPrivateIdentifier,
    UnterminatedRegExp,
    LineTerminatorInRegExp,
    ExpectedRegExp,
    ExpectedTemplateContinuation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} (byte {}): {}",
            self.span.start.line, self.span.start.column, self.span.start.byte_offset, self.message
        )
    }
}

impl std::error::Error for LexError {}

/// Stateful scanner over one UTF-8 source string.
pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: u32,
    column: u32,
    options: LexerOptions,
    eof_emitted: bool,
    failed: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self::with_options(source, LexerOptions::default())
    }

    pub fn with_options(source: &'a str, options: LexerOptions) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
            options,
            eof_emitted: false,
            failed: false,
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn current_position(&self) -> Position {
        Position::new(self.offset, self.line, self.column)
    }

    pub fn context(&self) -> LexContext {
        self.options.context
    }

    /// A parser may update context before scanning a contextual keyword.
    pub fn set_context(&mut self, context: LexContext) {
        self.options.context = context;
    }

    /// Scans with the division lexical goal and includes one EOF token.
    ///
    /// A parser must use next_token_with_goal for regular expressions and
    /// template continuations. This convenience method deliberately does not
    /// guess expression context.
    pub fn tokenize(mut self) -> Result<Vec<Token<'a>>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = matches!(token.kind, TokenKind::Eof);
            tokens.push(token);
            if is_eof {
                return Ok(tokens);
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token<'a>, LexError> {
        self.next_token_with_goal(LexicalGoal::Div)
    }

    pub fn next_token_with_goal(&mut self, goal: LexicalGoal) -> Result<Token<'a>, LexError> {
        if goal == LexicalGoal::TemplateContinuation {
            return self.scan_template(false, false);
        }

        let line_terminator_before = self.skip_trivia()?;
        let start = self.current_position();

        if self.offset == self.source.len() {
            self.eof_emitted = true;
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
                line_terminator_before,
            });
        }

        if goal == LexicalGoal::RegExp {
            if self.peek_char() == Some('/') {
                return self.scan_regexp(start, line_terminator_before);
            }
            return Err(self.error_here(
                LexErrorKind::ExpectedRegExp,
                "regular-expression lexical goal requires a slash",
            ));
        }

        let ch = self.peek_char().expect("checked non-empty source");
        let kind = match ch {
            '\'' | '"' => self.scan_string()?,
            c if c == TEMPLATE_QUOTE => {
                return self.scan_template(true, line_terminator_before);
            }
            '0'..='9' => self.scan_number(false)?,
            '.' if self.peek_nth_char(1).is_some_and(|c| c.is_ascii_digit()) => {
                self.scan_number(true)?
            }
            c if is_ascii_identifier_start(c) || c == '\\' => self.scan_identifier(false)?,
            '#' => self.scan_identifier(true)?,
            c if !c.is_ascii() => {
                return Err(self.unsupported_unicode_identifier(c));
            }
            _ => TokenKind::Punctuator(self.scan_punctuator()?),
        };

        Ok(Token {
            kind,
            span: Span::new(start, self.current_position()),
            line_terminator_before,
        })
    }

    fn peek_char(&self) -> Option<char> {
        self.source.get(self.offset..)?.chars().next()
    }

    fn peek_nth_char(&self, n: usize) -> Option<char> {
        self.source.get(self.offset..)?.chars().nth(n)
    }

    fn starts_with(&self, text: &str) -> bool {
        self.source
            .get(self.offset..)
            .is_some_and(|rest| rest.starts_with(text))
    }

    /// Advances one scalar value, treating CRLF as one logical character.
    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        if ch == '\r' && self.source[self.offset..].starts_with("\r\n") {
            self.offset += 2;
            self.line = self.line.saturating_add(1);
            self.column = 1;
            return Some('\r');
        }

        self.offset += ch.len_utf8();
        if is_line_terminator(ch) {
            self.line = self.line.saturating_add(1);
            self.column = 1;
        } else {
            self.column = self.column.saturating_add(1);
        }
        Some(ch)
    }

    fn consume_ascii(&mut self, text: &str) {
        debug_assert!(text.is_ascii());
        debug_assert!(self.starts_with(text));
        for _ in text.bytes() {
            let consumed = self.bump_char();
            debug_assert!(consumed.is_some());
        }
    }

    fn error_from(
        &self,
        start: Position,
        kind: LexErrorKind,
        message: impl Into<String>,
    ) -> LexError {
        LexError {
            kind,
            span: Span::new(start, self.current_position()),
            message: message.into(),
        }
    }

    fn error_here(&self, kind: LexErrorKind, message: impl Into<String>) -> LexError {
        let start = self.current_position();
        let mut end = start;
        if let Some(ch) = self.peek_char() {
            end.byte_offset += if ch == '\r' && self.starts_with("\r\n") {
                2
            } else {
                ch.len_utf8()
            };
            if is_line_terminator(ch) {
                end.line = end.line.saturating_add(1);
                end.column = 1;
            } else {
                end.column = end.column.saturating_add(1);
            }
        }
        LexError {
            kind,
            span: Span::new(start, end),
            message: message.into(),
        }
    }

    fn unsupported_unicode_identifier(&self, ch: char) -> LexError {
        self.error_here(
            LexErrorKind::UnsupportedUnicodeIdentifier,
            format!(
                "non-ASCII identifier character U+{:04X} is not supported yet",
                ch as u32
            ),
        )
    }

    fn skip_trivia(&mut self) -> Result<bool, LexError> {
        let mut saw_line_terminator = false;
        let token_search_start = self.offset;

        loop {
            let Some(ch) = self.peek_char() else {
                return Ok(saw_line_terminator);
            };

            if self.offset == 0 && self.starts_with("#!") {
                self.consume_ascii("#!");
                self.skip_to_line_end();
                continue;
            }

            if is_line_terminator(ch) {
                self.bump_char();
                saw_line_terminator = true;
                continue;
            }

            if is_js_whitespace(ch) {
                self.bump_char();
                continue;
            }

            if self.starts_with("//") {
                self.consume_ascii("//");
                self.skip_to_line_end();
                continue;
            }

            if self.starts_with("/*") {
                let comment_start = self.current_position();
                self.consume_ascii("/*");
                let mut terminated = false;
                while self.offset < self.source.len() {
                    if self.starts_with("*/") {
                        self.consume_ascii("*/");
                        terminated = true;
                        break;
                    }
                    let next = self.peek_char().expect("not at end");
                    if is_line_terminator(next) {
                        saw_line_terminator = true;
                    }
                    self.bump_char();
                }
                if !terminated {
                    return Err(self.error_from(
                        comment_start,
                        LexErrorKind::UnterminatedComment,
                        "unterminated block comment",
                    ));
                }
                continue;
            }

            if self.options.allow_html_comments && self.starts_with("<!--") {
                self.consume_ascii("<!--");
                self.skip_to_line_end();
                continue;
            }

            let at_line_start = saw_line_terminator || token_search_start == 0;
            if self.options.allow_html_comments && at_line_start && self.starts_with("-->") {
                self.consume_ascii("-->");
                self.skip_to_line_end();
                continue;
            }

            return Ok(saw_line_terminator);
        }
    }

    fn skip_to_line_end(&mut self) {
        while let Some(ch) = self.peek_char() {
            if is_line_terminator(ch) {
                break;
            }
            self.bump_char();
        }
    }

    fn scan_identifier(&mut self, private: bool) -> Result<TokenKind<'a>, LexError> {
        let start = self.current_position();
        let raw_start = self.offset;
        if private {
            self.bump_char();
        }

        let mut value = String::new();
        let mut has_escape = false;
        let mut first = true;

        loop {
            let Some(ch) = self.peek_char() else {
                if first {
                    return Err(self.error_from(
                        start,
                        LexErrorKind::InvalidPrivateIdentifier,
                        "private identifier is missing a name",
                    ));
                }
                break;
            };

            if ch == '\\' {
                let escape_start = self.current_position();
                let decoded = self.scan_identifier_escape()?;
                let valid = if first {
                    is_ascii_identifier_start(decoded)
                } else {
                    is_ascii_identifier_continue(decoded)
                };
                if !decoded.is_ascii() {
                    return Err(self.error_from(
                        escape_start,
                        LexErrorKind::UnsupportedUnicodeIdentifier,
                        format!(
                            "identifier escape resolves to unsupported U+{:04X}",
                            decoded as u32
                        ),
                    ));
                }
                if !valid {
                    return Err(self.error_from(
                        escape_start,
                        if private && first {
                            LexErrorKind::InvalidPrivateIdentifier
                        } else {
                            LexErrorKind::UnexpectedCharacter
                        },
                        "Unicode escape is not valid at this identifier position",
                    ));
                }
                value.push(decoded);
                has_escape = true;
                first = false;
                continue;
            }

            let valid = if first {
                is_ascii_identifier_start(ch)
            } else {
                is_ascii_identifier_continue(ch)
            };
            if valid {
                self.bump_char();
                value.push(ch);
                first = false;
                continue;
            }

            // Non-ASCII whitespace and U+2028/U+2029 terminate an ASCII
            // identifier just like their ASCII counterparts. They must be
            // left for `skip_trivia`, which records the restricted-production
            // LineTerminator flag used by postfix updates.
            if !first && (is_line_terminator(ch) || is_js_whitespace(ch)) {
                break;
            }
            if !ch.is_ascii() {
                return Err(self.unsupported_unicode_identifier(ch));
            }
            if first {
                return Err(self.error_from(
                    start,
                    if private {
                        LexErrorKind::InvalidPrivateIdentifier
                    } else {
                        LexErrorKind::UnexpectedCharacter
                    },
                    if private {
                        "invalid first character of private identifier"
                    } else {
                        "invalid first character of identifier"
                    },
                ));
            }
            break;
        }

        let raw = &self.source[raw_start..self.offset];
        let keyword_hint = keyword_from_str(&value);
        let active_keyword = keyword_hint.filter(|keyword| self.keyword_is_active(*keyword));
        let identifier = Identifier {
            raw,
            value,
            has_escape,
            keyword_hint,
            escaped_reserved_word: has_escape && active_keyword.is_some(),
        };

        if private {
            Ok(TokenKind::PrivateIdentifier(identifier))
        } else if !has_escape {
            if let Some(keyword) = active_keyword {
                Ok(TokenKind::Keyword(keyword))
            } else {
                Ok(TokenKind::Identifier(identifier))
            }
        } else {
            Ok(TokenKind::Identifier(identifier))
        }
    }

    fn scan_identifier_escape(&mut self) -> Result<char, LexError> {
        let start = self.current_position();
        self.bump_char();
        if self.peek_char() != Some('u') {
            return Err(self.error_from(
                start,
                LexErrorKind::InvalidEscape,
                "identifier escapes must use a Unicode escape",
            ));
        }
        self.bump_char();
        let value = self.scan_unicode_escape_value(start)?;
        char::from_u32(value).ok_or_else(|| {
            self.error_from(
                start,
                LexErrorKind::InvalidEscape,
                "identifier escape is not a Unicode scalar value",
            )
        })
    }

    fn keyword_is_active(&self, keyword: Keyword) -> bool {
        use Keyword::*;
        match keyword {
            Implements | Interface | Let | Package | Private | Protected | Public | Static => {
                self.options.context.strict
            }
            Yield => self.options.context.strict || self.options.context.generator,
            Await => self.options.context.module || self.options.context.async_function,
            _ => true,
        }
    }

    fn scan_number(&mut self, initial_dot: bool) -> Result<TokenKind<'a>, LexError> {
        let start = self.current_position();
        let raw_start = self.offset;

        if !initial_dot && self.peek_char() == Some('0') {
            let prefix = self.peek_nth_char(1);
            let radix = match prefix {
                Some('b' | 'B') => Some(NumericRadix::Binary),
                Some('o' | 'O') => Some(NumericRadix::Octal),
                Some('x' | 'X') => Some(NumericRadix::Hexadecimal),
                _ => None,
            };
            if let Some(radix) = radix {
                self.bump_char();
                self.bump_char();
                let base = radix_value(radix);
                let run = self.scan_digit_run(base)?;
                if run.digits == 0 {
                    return Err(self.error_from(
                        start,
                        LexErrorKind::InvalidNumber,
                        format!("base-{base} literal requires at least one digit"),
                    ));
                }

                let kind = if self.peek_char() == Some('n') {
                    self.bump_char();
                    NumberKind::BigInt(radix)
                } else {
                    NumberKind::Integer(radix)
                };
                self.reject_number_follow(start)?;
                return Ok(TokenKind::Number(NumberLiteral {
                    raw: &self.source[raw_start..self.offset],
                    kind,
                }));
            }
        }

        let mut is_float = initial_dot;
        let mut had_separator = false;
        let mut integer_digits = 0;
        let mut leading_zero = false;
        let mut saw_8_or_9 = false;
        let mut legacy_octal_integer = false;

        if initial_dot {
            self.bump_char();
            let run = self.scan_digit_run(10)?;
            if run.digits == 0 {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "fraction requires a decimal digit",
                ));
            }
            had_separator |= run.had_separator;
        } else {
            leading_zero = self.peek_char() == Some('0');
            if leading_zero && self.peek_nth_char(1) == Some('_') {
                self.bump_char();
                self.bump_char();
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "numeric separator cannot follow a leading zero",
                ));
            }
            let run = self.scan_digit_run(10)?;
            integer_digits = run.digits;
            had_separator |= run.had_separator;
            saw_8_or_9 |= run.saw_8_or_9;
            legacy_octal_integer = leading_zero && integer_digits > 1 && !saw_8_or_9;
            if leading_zero && integer_digits > 1 && had_separator {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "numeric separators are not allowed in legacy leading-zero literals",
                ));
            }

            if self.peek_char() == Some('.') && !legacy_octal_integer {
                is_float = true;
                self.bump_char();
                let run = self.scan_digit_run(10)?;
                had_separator |= run.had_separator;
            }
        }

        if matches!(self.peek_char(), Some('e' | 'E')) && !legacy_octal_integer {
            is_float = true;
            self.bump_char();
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.bump_char();
            }
            let run = self.scan_digit_run(10)?;
            if run.digits == 0 {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "exponent requires at least one decimal digit",
                ));
            }
            had_separator |= run.had_separator;
        }

        let legacy_leading_zero = leading_zero && integer_digits > 1;
        if legacy_leading_zero && had_separator {
            return Err(self.error_from(
                start,
                LexErrorKind::InvalidNumber,
                "numeric separators are not allowed in legacy leading-zero literals",
            ));
        }
        if legacy_leading_zero && self.options.context.strict {
            return Err(self.error_from(
                start,
                LexErrorKind::InvalidNumber,
                "legacy leading-zero numeric literals are not allowed in strict mode",
            ));
        }

        let kind = if self.peek_char() == Some('n') {
            if is_float {
                self.bump_char();
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "BigInt suffix cannot follow a fraction or exponent",
                ));
            }
            if legacy_leading_zero {
                self.bump_char();
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidNumber,
                    "decimal BigInt cannot contain a leading zero",
                ));
            }
            self.bump_char();
            NumberKind::BigInt(NumericRadix::Decimal)
        } else if is_float {
            NumberKind::Float
        } else if legacy_leading_zero && !had_separator {
            if saw_8_or_9 {
                NumberKind::LegacyDecimal
            } else {
                NumberKind::LegacyOctal
            }
        } else {
            NumberKind::Integer(NumericRadix::Decimal)
        };

        self.reject_number_follow(start)?;
        Ok(TokenKind::Number(NumberLiteral {
            raw: &self.source[raw_start..self.offset],
            kind,
        }))
    }

    fn scan_digit_run(&mut self, base: u32) -> Result<DigitRun, LexError> {
        let mut result = DigitRun::default();
        let mut previous_was_digit = false;

        while let Some(ch) = self.peek_char() {
            if ch.is_digit(base) {
                self.bump_char();
                result.digits += 1;
                result.saw_8_or_9 |= matches!(ch, '8' | '9');
                previous_was_digit = true;
                continue;
            }

            if ch == '_' {
                let separator = self.current_position();
                let next_is_digit = self
                    .peek_nth_char(1)
                    .is_some_and(|next| next.is_digit(base));
                if !previous_was_digit || !next_is_digit {
                    self.bump_char();
                    return Err(self.error_from(
                        separator,
                        LexErrorKind::InvalidNumber,
                        "numeric separator must occur between two digits",
                    ));
                }
                self.bump_char();
                result.had_separator = true;
                previous_was_digit = false;
                continue;
            }
            break;
        }
        Ok(result)
    }

    fn reject_number_follow(&mut self, start: Position) -> Result<(), LexError> {
        let Some(ch) = self.peek_char() else {
            return Ok(());
        };
        if is_line_terminator(ch) || is_js_whitespace(ch) {
            return Ok(());
        }
        if is_ascii_identifier_continue(ch) || ch == '\\' || !ch.is_ascii() {
            self.bump_char();
            return Err(self.error_from(
                start,
                LexErrorKind::InvalidNumber,
                "identifier characters may not immediately follow a number literal",
            ));
        }
        Ok(())
    }

    fn scan_string(&mut self) -> Result<TokenKind<'a>, LexError> {
        let start = self.current_position();
        let raw_start = self.offset;
        let separator = self.bump_char().expect("called at quote");
        let quote = if separator == '\'' {
            Quote::Single
        } else {
            Quote::Double
        };
        let mut value = JsString::new();
        let mut has_escape = false;
        let mut has_legacy_octal_escape = false;

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_from(
                    start,
                    LexErrorKind::UnterminatedString,
                    "unterminated string literal",
                ));
            };
            if ch == separator {
                self.bump_char();
                return Ok(TokenKind::String(StringLiteral {
                    raw: &self.source[raw_start..self.offset],
                    value,
                    quote,
                    has_escape,
                    has_legacy_octal_escape,
                }));
            }
            if matches!(ch, '\r' | '\n') {
                return Err(self.error_here(
                    LexErrorKind::UnterminatedString,
                    "unescaped line terminator in string literal",
                ));
            }
            if ch == '\\' {
                has_escape = true;
                let escape = self.scan_escape_sequence(false)?;
                has_legacy_octal_escape |= escape.legacy_octal;
                if let Some(code_point) = escape.code_point {
                    value.push_code_point(code_point);
                }
                continue;
            }

            self.bump_char();
            value.push_char(ch);
        }
    }

    fn scan_escape_sequence(&mut self, template: bool) -> Result<EscapeValue, LexError> {
        let start = self.current_position();
        debug_assert_eq!(self.peek_char(), Some('\\'));
        self.bump_char();
        let Some(ch) = self.peek_char() else {
            return Err(self.error_from(
                start,
                LexErrorKind::InvalidEscape,
                "escape sequence reaches end of source",
            ));
        };

        if is_line_terminator(ch) {
            self.bump_char();
            return Ok(EscapeValue::line_continuation());
        }

        let simple = match ch {
            '\'' => Some('\'' as u32),
            '"' => Some('"' as u32),
            '\\' => Some('\\' as u32),
            'b' => Some(0x08),
            'f' => Some(0x0c),
            'n' => Some('\n' as u32),
            'r' => Some('\r' as u32),
            't' => Some('\t' as u32),
            'v' => Some(0x0b),
            _ => None,
        };
        if let Some(code_point) = simple {
            self.bump_char();
            return Ok(EscapeValue::code_point(code_point));
        }

        if ch == '0'
            && !self
                .peek_nth_char(1)
                .is_some_and(|next| next.is_ascii_digit())
        {
            self.bump_char();
            return Ok(EscapeValue::code_point(0));
        }

        match ch {
            'x' => {
                self.bump_char();
                let value = self.scan_fixed_hex_digits(2, start)?;
                Ok(EscapeValue::code_point(value))
            }
            'u' => {
                self.bump_char();
                let value = self.scan_unicode_escape_value(start)?;
                Ok(EscapeValue::code_point(value))
            }
            '0'..='7' => {
                if template || self.options.context.strict {
                    self.bump_char();
                    return Err(self.error_from(
                        start,
                        LexErrorKind::InvalidEscape,
                        "legacy octal escape is not allowed in this context",
                    ));
                }
                let value = self.scan_legacy_octal_escape();
                Ok(EscapeValue::legacy(value))
            }
            '8' | '9' => {
                if template || self.options.context.strict {
                    self.bump_char();
                    return Err(self.error_from(
                        start,
                        LexErrorKind::InvalidEscape,
                        "escape 8 or 9 is not allowed in strict strings or templates",
                    ));
                }
                self.bump_char();
                Ok(EscapeValue::legacy(ch as u32))
            }
            _ => {
                self.bump_char();
                Ok(EscapeValue::code_point(ch as u32))
            }
        }
    }

    fn scan_legacy_octal_escape(&mut self) -> u32 {
        let first = self.peek_char().expect("called at octal digit");
        let max_digits = if matches!(first, '0'..='3') { 3 } else { 2 };
        let mut value = 0_u32;
        let mut count = 0;
        while count < max_digits {
            let Some(ch @ '0'..='7') = self.peek_char() else {
                break;
            };
            self.bump_char();
            value = (value * 8) + ch.to_digit(8).expect("matched octal digit");
            count += 1;
        }
        value
    }

    fn scan_unicode_escape_value(&mut self, start: Position) -> Result<u32, LexError> {
        if self.peek_char() == Some('{') {
            self.bump_char();
            let mut digits = 0;
            let mut value = 0_u32;
            while let Some(ch) = self.peek_char() {
                let Some(digit) = ch.to_digit(16) else {
                    break;
                };
                if digits == 6 {
                    self.bump_char();
                    return Err(self.error_from(
                        start,
                        LexErrorKind::InvalidEscape,
                        "Unicode code point escape has more than six hex digits",
                    ));
                }
                self.bump_char();
                value = (value << 4) | digit;
                digits += 1;
            }
            if digits == 0 || self.peek_char() != Some('}') {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidEscape,
                    "malformed braced Unicode escape",
                ));
            }
            self.bump_char();
            if value > 0x10ffff {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidEscape,
                    "Unicode escape exceeds U+10FFFF",
                ));
            }
            Ok(value)
        } else {
            self.scan_fixed_hex_digits(4, start)
        }
    }

    fn scan_fixed_hex_digits(&mut self, count: usize, start: Position) -> Result<u32, LexError> {
        let mut value = 0_u32;
        for _ in 0..count {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidEscape,
                    format!("escape requires exactly {count} hexadecimal digits"),
                ));
            };
            let Some(digit) = ch.to_digit(16) else {
                self.bump_char();
                return Err(self.error_from(
                    start,
                    LexErrorKind::InvalidEscape,
                    format!("escape requires exactly {count} hexadecimal digits"),
                ));
            };
            self.bump_char();
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    fn scan_template(
        &mut self,
        initial: bool,
        line_terminator_before: bool,
    ) -> Result<Token<'a>, LexError> {
        let start = self.current_position();
        if initial {
            debug_assert_eq!(self.peek_char(), Some(TEMPLATE_QUOTE));
            self.bump_char();
        }
        let raw_start = self.offset;
        let mut cooked = Some(JsString::new());
        let mut invalid_escape = None;

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_from(
                    start,
                    LexErrorKind::UnterminatedTemplate,
                    "unterminated template literal",
                ));
            };

            if ch == TEMPLATE_QUOTE {
                let raw_end = self.offset;
                self.bump_char();
                let kind = if initial {
                    TemplatePartKind::NoSubstitution
                } else {
                    TemplatePartKind::Tail
                };
                return Ok(Token {
                    kind: TokenKind::Template(TemplatePart {
                        raw: &self.source[raw_start..raw_end],
                        raw_value: template_raw_value(&self.source[raw_start..raw_end]),
                        cooked,
                        invalid_escape,
                        kind,
                    }),
                    span: Span::new(start, self.current_position()),
                    line_terminator_before,
                });
            }

            if ch == '$' && self.peek_nth_char(1) == Some('{') {
                let raw_end = self.offset;
                self.bump_char();
                self.bump_char();
                let kind = if initial {
                    TemplatePartKind::Head
                } else {
                    TemplatePartKind::Middle
                };
                return Ok(Token {
                    kind: TokenKind::Template(TemplatePart {
                        raw: &self.source[raw_start..raw_end],
                        raw_value: template_raw_value(&self.source[raw_start..raw_end]),
                        cooked,
                        invalid_escape,
                        kind,
                    }),
                    span: Span::new(start, self.current_position()),
                    line_terminator_before,
                });
            }

            if ch == '\\' {
                match self.scan_escape_sequence(true) {
                    Ok(escape) => {
                        if let (Some(value), Some(output)) = (escape.code_point, cooked.as_mut()) {
                            output.push_code_point(value);
                        }
                    }
                    Err(error) => {
                        if invalid_escape.is_none() {
                            invalid_escape = Some(TemplateEscapeError {
                                message: error.message,
                                span: error.span,
                            });
                        }
                        cooked = None;
                    }
                }
                continue;
            }

            self.bump_char();
            if let Some(output) = cooked.as_mut() {
                if ch == '\r' {
                    output.push_char('\n');
                } else {
                    output.push_char(ch);
                }
            }
        }
    }

    fn scan_regexp(
        &mut self,
        start: Position,
        line_terminator_before: bool,
    ) -> Result<Token<'a>, LexError> {
        let raw_start = self.offset;
        debug_assert_eq!(self.peek_char(), Some('/'));
        self.bump_char();
        let pattern_start = self.offset;
        let mut in_character_class = false;

        let pattern_end = loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_from(
                    start,
                    LexErrorKind::UnterminatedRegExp,
                    "unterminated regular-expression literal",
                ));
            };
            if is_line_terminator(ch) {
                return Err(self.error_here(
                    LexErrorKind::LineTerminatorInRegExp,
                    "line terminator is not allowed in a regular-expression literal",
                ));
            }
            if ch == '/' && !in_character_class {
                let end = self.offset;
                self.bump_char();
                break end;
            }
            if ch == '\\' {
                self.bump_char();
                let Some(escaped) = self.peek_char() else {
                    return Err(self.error_from(
                        start,
                        LexErrorKind::UnterminatedRegExp,
                        "regular-expression escape reaches end of source",
                    ));
                };
                if is_line_terminator(escaped) {
                    return Err(self.error_here(
                        LexErrorKind::LineTerminatorInRegExp,
                        "escaped line terminator is not allowed in a regular expression",
                    ));
                }
                self.bump_char();
                continue;
            }
            if ch == '[' {
                in_character_class = true;
            } else if ch == ']' {
                in_character_class = false;
            }
            self.bump_char();
        };

        let flags_start = self.offset;
        while let Some(ch) = self.peek_char() {
            if is_ascii_identifier_continue(ch) {
                self.bump_char();
            } else if !ch.is_ascii() {
                return Err(self.unsupported_unicode_identifier(ch));
            } else {
                break;
            }
        }

        Ok(Token {
            kind: TokenKind::RegExp(RegExpLiteral {
                raw: &self.source[raw_start..self.offset],
                pattern: &self.source[pattern_start..pattern_end],
                flags: &self.source[flags_start..self.offset],
            }),
            span: Span::new(start, self.current_position()),
            line_terminator_before,
        })
    }

    fn scan_punctuator(&mut self) -> Result<Punctuator, LexError> {
        use Punctuator::*;

        if self.starts_with("?.") && !self.peek_nth_char(2).is_some_and(|ch| ch.is_ascii_digit()) {
            self.consume_ascii("?.");
            return Ok(OptionalChain);
        }

        const PUNCTUATORS: &[(&str, Punctuator)] = &[
            (">>>=", UnsignedShiftRightAssign),
            ("===", StrictEqual),
            ("!==", StrictNotEqual),
            ("**=", ExponentAssign),
            ("<<=", ShiftLeftAssign),
            (">>=", ShiftRightAssign),
            ("&&=", LogicalAndAssign),
            ("||=", LogicalOrAssign),
            ("??=", NullishAssign),
            (">>>", UnsignedShiftRight),
            ("...", Ellipsis),
            ("=>", Arrow),
            ("==", EqualEqual),
            ("!=", NotEqual),
            ("<=", LessEqual),
            (">=", GreaterEqual),
            ("++", Increment),
            ("--", Decrement),
            ("**", Exponent),
            ("<<", ShiftLeft),
            (">>", ShiftRight),
            ("&&", LogicalAnd),
            ("||", LogicalOr),
            ("??", NullishCoalesce),
            ("+=", PlusAssign),
            ("-=", MinusAssign),
            ("*=", MultiplyAssign),
            ("/=", DivideAssign),
            ("%=", RemainderAssign),
            ("&=", BitAndAssign),
            ("|=", BitOrAssign),
            ("^=", BitXorAssign),
            ("{", LeftBrace),
            ("}", RightBrace),
            ("(", LeftParen),
            (")", RightParen),
            ("[", LeftBracket),
            ("]", RightBracket),
            (".", Dot),
            (";", Semicolon),
            (",", Comma),
            ("<", Less),
            (">", Greater),
            ("=", Equal),
            ("!", Not),
            ("+", Plus),
            ("-", Minus),
            ("*", Multiply),
            ("/", Divide),
            ("%", Remainder),
            ("&", BitAnd),
            ("|", BitOr),
            ("^", BitXor),
            ("~", BitNot),
            ("?", Question),
            (":", Colon),
        ];

        for (text, punctuator) in PUNCTUATORS {
            if self.starts_with(text) {
                self.consume_ascii(text);
                return Ok(*punctuator);
            }
        }

        let ch = self.peek_char().expect("called before end of source");
        Err(self.error_here(
            LexErrorKind::UnexpectedCharacter,
            format!("unexpected character U+{:04X}", ch as u32),
        ))
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Token<'a>, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_emitted || self.failed {
            return None;
        }
        let result = self.next_token();
        if result.is_err() {
            self.failed = true;
        }
        Some(result)
    }
}

impl FusedIterator for Lexer<'_> {}

#[derive(Clone, Copy, Debug, Default)]
struct DigitRun {
    digits: usize,
    had_separator: bool,
    saw_8_or_9: bool,
}

#[derive(Clone, Copy, Debug)]
struct EscapeValue {
    code_point: Option<u32>,
    legacy_octal: bool,
}

impl EscapeValue {
    const fn code_point(value: u32) -> Self {
        Self {
            code_point: Some(value),
            legacy_octal: false,
        }
    }

    const fn legacy(value: u32) -> Self {
        Self {
            code_point: Some(value),
            legacy_octal: true,
        }
    }

    const fn line_continuation() -> Self {
        Self {
            code_point: None,
            legacy_octal: false,
        }
    }
}

fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\u{000b}' | '\u{000c}' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

fn is_ascii_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '_' | '$')
}

fn is_ascii_identifier_continue(ch: char) -> bool {
    is_ascii_identifier_start(ch) || ch.is_ascii_digit()
}

fn radix_value(radix: NumericRadix) -> u32 {
    match radix {
        NumericRadix::Binary => 2,
        NumericRadix::Octal => 8,
        NumericRadix::Decimal => 10,
        NumericRadix::Hexadecimal => 16,
    }
}

fn template_raw_value(source: &str) -> JsString {
    let mut value = JsString::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            value.push_char('\n');
        } else {
            value.push_char(ch);
        }
    }
    value
}

fn keyword_from_str(value: &str) -> Option<Keyword> {
    use Keyword::*;
    Some(match value {
        "null" => Null,
        "false" => False,
        "true" => True,
        "if" => If,
        "else" => Else,
        "return" => Return,
        "var" => Var,
        "this" => This,
        "delete" => Delete,
        "void" => Void,
        "typeof" => Typeof,
        "new" => New,
        "in" => In,
        "instanceof" => Instanceof,
        "do" => Do,
        "while" => While,
        "for" => For,
        "break" => Break,
        "continue" => Continue,
        "switch" => Switch,
        "case" => Case,
        "default" => Default,
        "throw" => Throw,
        "try" => Try,
        "catch" => Catch,
        "finally" => Finally,
        "function" => Function,
        "debugger" => Debugger,
        "with" => With,
        "class" => Class,
        "const" => Const,
        "enum" => Enum,
        "export" => Export,
        "extends" => Extends,
        "import" => Import,
        "super" => Super,
        "implements" => Implements,
        "interface" => Interface,
        "let" => Let,
        "package" => Package,
        "private" => Private,
        "protected" => Protected,
        "public" => Public,
        "static" => Static,
        "yield" => Yield,
        "await" => Await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind<'_>> {
        Lexer::new(source)
            .tokenize()
            .expect("source should tokenize")
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    fn number_kinds(source: &str) -> Vec<(&str, NumberKind)> {
        Lexer::new(source)
            .tokenize()
            .expect("numbers should tokenize")
            .into_iter()
            .filter_map(|token| match token.kind {
                TokenKind::Number(number) => Some((number.raw, number.kind)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn tracks_utf8_spans_lines_and_asi_trivia() {
        let source = "\"é\";\r\n/* one\n two */ const x = 1";
        let tokens = Lexer::new(source).tokenize().unwrap();

        assert_eq!(tokens[0].span.start, Position::new(0, 1, 1));
        assert_eq!(tokens[0].span.end, Position::new(4, 1, 4));
        assert!(matches!(tokens[2].kind, TokenKind::Keyword(Keyword::Const)));
        assert!(tokens[2].line_terminator_before);
        assert_eq!(tokens[2].span.start.line, 3);
        assert_eq!(tokens[2].span.start.column, 9);
        assert!(!tokens[3].line_terminator_before);
    }

    #[test]
    fn classifies_keywords_with_quickjs_context_rules() {
        let tokens = Lexer::new("const let yield await").tokenize().unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Keyword(Keyword::Const)));
        for token in &tokens[1..4] {
            assert!(matches!(
                token.kind,
                TokenKind::Identifier(Identifier {
                    keyword_hint: Some(_),
                    ..
                })
            ));
        }

        let options = LexerOptions {
            context: LexContext {
                strict: true,
                module: true,
                ..LexContext::default()
            },
            ..LexerOptions::default()
        };
        let tokens = Lexer::with_options("let yield await", options)
            .tokenize()
            .unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Keyword(Keyword::Let)));
        assert!(matches!(tokens[1].kind, TokenKind::Keyword(Keyword::Yield)));
        assert!(matches!(tokens[2].kind, TokenKind::Keyword(Keyword::Await)));
    }

    #[test]
    fn escaped_keyword_remains_identifier_with_reserved_marker() {
        let token = Lexer::new(r"\u0069f").next_token().unwrap();
        match token.kind {
            TokenKind::Identifier(identifier) => {
                assert_eq!(identifier.value, "if");
                assert!(identifier.has_escape);
                assert_eq!(identifier.keyword_hint, Some(Keyword::If));
                assert!(identifier.escaped_reserved_word);
            }
            other => panic!("expected identifier, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_ascii_identifier_without_panicking() {
        let error = Lexer::new("π = 1").next_token().unwrap_err();
        assert_eq!(error.kind, LexErrorKind::UnsupportedUnicodeIdentifier);
        assert_eq!(error.span.start, Position::new(0, 1, 1));
        assert!(error.to_string().contains("U+03C0"));

        let error = Lexer::new("asciiπ").next_token().unwrap_err();
        assert_eq!(error.kind, LexErrorKind::UnsupportedUnicodeIdentifier);
        assert_eq!(error.span.start.byte_offset, 5);
    }

    #[test]
    fn scans_private_identifiers_and_identifier_escapes() {
        let token = Lexer::new(r"#pr\u0069vate").next_token().unwrap();
        match token.kind {
            TokenKind::PrivateIdentifier(identifier) => {
                assert_eq!(identifier.raw, r"#pr\u0069vate");
                assert_eq!(identifier.value, "private");
                assert!(identifier.has_escape);
            }
            other => panic!("expected private identifier, got {other:?}"),
        }
        assert_eq!(
            Lexer::new("#1").next_token().unwrap_err().kind,
            LexErrorKind::InvalidPrivateIdentifier
        );
    }

    #[test]
    fn scans_numeric_radices_floats_exponents_separators_and_bigints() {
        assert_eq!(
            number_kinds("0 42 1_000 0b1010 0o755 0xCA_FE .5 1. 2e-3 99n 0xffn"),
            vec![
                ("0", NumberKind::Integer(NumericRadix::Decimal)),
                ("42", NumberKind::Integer(NumericRadix::Decimal)),
                ("1_000", NumberKind::Integer(NumericRadix::Decimal)),
                ("0b1010", NumberKind::Integer(NumericRadix::Binary)),
                ("0o755", NumberKind::Integer(NumericRadix::Octal)),
                ("0xCA_FE", NumberKind::Integer(NumericRadix::Hexadecimal)),
                (".5", NumberKind::Float),
                ("1.", NumberKind::Float),
                ("2e-3", NumberKind::Float),
                ("99n", NumberKind::BigInt(NumericRadix::Decimal)),
                ("0xffn", NumberKind::BigInt(NumericRadix::Hexadecimal)),
            ]
        );
        assert_eq!(
            number_kinds("077 089"),
            vec![
                ("077", NumberKind::LegacyOctal),
                ("089", NumberKind::LegacyDecimal),
            ]
        );
    }

    #[test]
    fn rejects_malformed_numbers() {
        for source in [
            "0b2",
            "0o8",
            "0x",
            "1__0",
            "1_",
            "1e+",
            "1.0n",
            "01n",
            "00_1",
            "08.1_2",
            "08e1_2",
            "10instanceof",
        ] {
            let error = Lexer::new(source).next_token().unwrap_err();
            assert_eq!(
                error.kind,
                LexErrorKind::InvalidNumber,
                "unexpected error for {source}"
            );
        }

        let options = LexerOptions {
            context: LexContext {
                strict: true,
                ..LexContext::default()
            },
            ..LexerOptions::default()
        };
        assert_eq!(
            Lexer::with_options("077", options)
                .next_token()
                .unwrap_err()
                .kind,
            LexErrorKind::InvalidNumber
        );
    }

    #[test]
    fn decodes_string_escapes_without_losing_lone_surrogates() {
        let source = r#""a\n\x42\u{43}\uD800" '\141\8\0'"#;
        let tokens = Lexer::new(source).tokenize().unwrap();
        let TokenKind::String(first) = &tokens[0].kind else {
            panic!("expected string");
        };
        assert_eq!(first.value.utf16, vec![0x61, 0x0a, 0x42, 0x43, 0xd800]);
        assert!(first.value.to_string().is_err());

        let TokenKind::String(second) = &tokens[1].kind else {
            panic!("expected string");
        };
        assert_eq!(second.value.utf16, vec![0x61, 0x38, 0]);
        assert!(second.has_legacy_octal_escape);
    }

    #[test]
    fn handles_string_line_continuations_and_strict_escape_errors() {
        let token = Lexer::new("\"a\\\r\nb\"").next_token().unwrap();
        let TokenKind::String(string) = token.kind else {
            panic!("expected string");
        };
        assert_eq!(string.value.to_string().unwrap(), "ab");
        assert_eq!(token.span.end.line, 2);

        let options = LexerOptions {
            context: LexContext {
                strict: true,
                ..LexContext::default()
            },
            ..LexerOptions::default()
        };
        assert_eq!(
            Lexer::with_options(r"'\1'", options)
                .next_token()
                .unwrap_err()
                .kind,
            LexErrorKind::InvalidEscape
        );
        let zero = Lexer::with_options(r"'\0'", options).next_token().unwrap();
        let TokenKind::String(zero) = zero.kind else {
            panic!("expected string");
        };
        assert_eq!(zero.value.utf16, vec![0]);
    }

    #[test]
    fn longest_matches_punctuators() {
        let tokens = kinds(">>>= === !== **= &&= ||= ??= ... ?. ?.3 => ++ --");
        let punctuators: Vec<Punctuator> = tokens
            .into_iter()
            .filter_map(|kind| match kind {
                TokenKind::Punctuator(punctuator) => Some(punctuator),
                _ => None,
            })
            .collect();
        assert_eq!(
            punctuators,
            vec![
                Punctuator::UnsignedShiftRightAssign,
                Punctuator::StrictEqual,
                Punctuator::StrictNotEqual,
                Punctuator::ExponentAssign,
                Punctuator::LogicalAndAssign,
                Punctuator::LogicalOrAssign,
                Punctuator::NullishAssign,
                Punctuator::Ellipsis,
                Punctuator::OptionalChain,
                Punctuator::Question,
                Punctuator::Arrow,
                Punctuator::Increment,
                Punctuator::Decrement,
            ]
        );
    }

    #[test]
    fn slash_interpretation_is_an_explicit_lexical_goal() {
        let mut division = Lexer::new("/= value");
        assert!(matches!(
            division.next_token().unwrap().kind,
            TokenKind::Punctuator(Punctuator::DivideAssign)
        ));

        let mut regexp = Lexer::new(r"/a[\/]b+/gim");
        let token = regexp.next_token_with_goal(LexicalGoal::RegExp).unwrap();
        match token.kind {
            TokenKind::RegExp(literal) => {
                assert_eq!(literal.pattern, r"a[\/]b+");
                assert_eq!(literal.flags, "gim");
                assert_eq!(literal.raw, r"/a[\/]b+/gim");
            }
            other => panic!("expected regular expression, got {other:?}"),
        }

        assert_eq!(
            Lexer::new("value")
                .next_token_with_goal(LexicalGoal::RegExp)
                .unwrap_err()
                .kind,
            LexErrorKind::ExpectedRegExp
        );
    }

    #[test]
    fn regexp_rejects_line_terminators() {
        let mut lexer = Lexer::new("/a\n/");
        assert_eq!(
            lexer
                .next_token_with_goal(LexicalGoal::RegExp)
                .unwrap_err()
                .kind,
            LexErrorKind::LineTerminatorInRegExp
        );
    }

    #[test]
    fn template_continuation_is_parser_driven() {
        let source = format!(
            "{}head{}{{name}}tail{}",
            TEMPLATE_QUOTE, '$', TEMPLATE_QUOTE
        );
        let mut lexer = Lexer::new(&source);
        let head = lexer.next_token().unwrap();
        let TokenKind::Template(head) = head.kind else {
            panic!("expected template head");
        };
        assert_eq!(head.kind, TemplatePartKind::Head);
        assert_eq!(head.raw, "head");
        assert_eq!(head.cooked.unwrap().to_string().unwrap(), "head");

        assert!(matches!(
            lexer.next_token().unwrap().kind,
            TokenKind::Identifier(_)
        ));
        assert!(matches!(
            lexer.next_token().unwrap().kind,
            TokenKind::Punctuator(Punctuator::RightBrace)
        ));

        let tail = lexer
            .next_token_with_goal(LexicalGoal::TemplateContinuation)
            .unwrap();
        let TokenKind::Template(tail) = tail.kind else {
            panic!("expected template tail");
        };
        assert_eq!(tail.kind, TemplatePartKind::Tail);
        assert_eq!(tail.raw, "tail");
        assert_eq!(tail.cooked.unwrap().to_string().unwrap(), "tail");
    }

    #[test]
    fn template_invalid_escape_is_preserved_for_tagged_semantics() {
        let source = format!("{}bad\\8{}", TEMPLATE_QUOTE, TEMPLATE_QUOTE);
        let token = Lexer::new(&source).next_token().unwrap();
        let TokenKind::Template(part) = token.kind else {
            panic!("expected template");
        };
        assert_eq!(part.kind, TemplatePartKind::NoSubstitution);
        assert!(part.cooked.is_none());
        let invalid = part.invalid_escape.expect("invalid escape metadata");
        assert!(invalid.message.contains("not allowed"));
    }

    #[test]
    fn template_raw_value_normalizes_only_physical_crlf() {
        let source = format!("{}a\r\n\\nb{}", TEMPLATE_QUOTE, TEMPLATE_QUOTE);
        let token = Lexer::new(&source).next_token().unwrap();
        let TokenKind::Template(part) = token.kind else {
            panic!("expected template");
        };
        assert_eq!(part.raw, "a\r\n\\nb");
        assert_eq!(part.raw_value.to_string().unwrap(), "a\n\\nb");
        assert_eq!(part.cooked.unwrap().to_string().unwrap(), "a\n\nb");
    }

    #[test]
    fn hashbang_and_comments_feed_line_terminator_metadata() {
        let source = "#!/usr/bin/env qjs\nconst a = 1 // comment\n++a";
        let tokens = Lexer::new(source).tokenize().unwrap();
        assert!(tokens[0].line_terminator_before);
        let increment = tokens
            .iter()
            .find(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Increment)))
            .unwrap();
        assert!(increment.line_terminator_before);
    }

    #[test]
    fn unicode_line_terminators_after_identifiers_feed_asi_metadata() {
        let tokens = Lexer::new("value\u{2028}++next\u{2029}--last")
            .tokenize()
            .unwrap();
        let updates = tokens
            .iter()
            .filter(|token| {
                matches!(
                    token.kind,
                    TokenKind::Punctuator(Punctuator::Increment | Punctuator::Decrement)
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 2);
        assert!(updates.iter().all(|token| token.line_terminator_before));
    }

    #[test]
    fn unterminated_constructs_have_readable_positions() {
        let comment = Lexer::new(" \n/* missing").next_token().unwrap_err();
        assert_eq!(comment.kind, LexErrorKind::UnterminatedComment);
        assert_eq!(comment.span.start.line, 2);
        assert_eq!(comment.span.start.column, 1);
        assert!(comment.to_string().starts_with("2:1"));

        let template_source = format!("{}missing", TEMPLATE_QUOTE);
        assert_eq!(
            Lexer::new(&template_source).next_token().unwrap_err().kind,
            LexErrorKind::UnterminatedTemplate
        );
    }

    #[test]
    fn iterator_yields_eof_once_and_then_fuses() {
        let mut lexer = Lexer::new("");
        let eof = lexer.next().unwrap().unwrap();
        assert!(matches!(eof.kind, TokenKind::Eof));
        assert!(lexer.next().is_none());
        assert!(lexer.next().is_none());
    }
}
