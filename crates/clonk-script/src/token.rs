#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
    /// Pointer-width unsigned spelling before C4Value's intentional i32
    /// truncation. Directive grammar uses this to distinguish a literal 2/3
    /// from a wider integer whose low 32 bits happen to equal 2/3.
    raw_number: Option<u64>,
    /// Whether a numeric token used C4Aul's lowercase-`0x` spelling. The
    /// static-constant preparser accepts hexadecimal integers directly, but
    /// its special signed-integer scan never enters hexadecimal mode.
    number_is_hex: bool,
    /// C4Aul's read position at the token's first byte and at the byte after
    /// it. The lexer stamps both.
    read_start: DiagnosticPosition,
    read_end: DiagnosticPosition,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number(i32),
    String(String),
    C4Id(String), // 4-character definition ID like "CLNK", "COWB"
    /// Strict-3 adjacent `global->` (C4Aul ATT_GLOBALCALL). The lexer
    /// consumes the arrow so whitespace keeps the ordinary identifier path.
    GlobalCall,
    Keyword(Keyword),
    Symbol(Symbol),
    Directive(String), // Directive like "#include", "#appendto", "#strict"
    LocaleKey(String), // Localization key like $TxtPermanentModeTurnOn$
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Global,
    Private,
    Protected,
    Public,
    Local,
    Var,
    Static,
    Const,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Break,
    Continue,
    True,
    False,
    Nil,
    This,
}

impl Keyword {
    /// The source text of the keyword. C4Aul keywords are contextual (the
    /// C++ tokenizer emits plain ATT_IDTF words), so parse positions that
    /// accept arbitrary identifiers — parameter names, expression variables
    /// — fall back to this lexeme.
    pub fn lexeme(self) -> &'static str {
        match self {
            Keyword::Global => "global",
            Keyword::Private => "private",
            Keyword::Protected => "protected",
            Keyword::Public => "public",
            Keyword::Local => "local",
            Keyword::Var => "var",
            Keyword::Static => "static",
            Keyword::Const => "const",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::While => "while",
            Keyword::For => "for",
            Keyword::In => "in",
            Keyword::Return => "return",
            Keyword::Break => "break",
            Keyword::Continue => "continue",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Nil => "nil",
            Keyword::This => "this",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,
    ColonColon, // :: (scope resolution)
    Plus,
    PlusPlus,
    Minus,
    MinusMinus,
    Star,
    StarStar,
    Slash,
    Percent,
    Dot,
    Ellipsis,              // ... (varargs forwarder)
    Concat,                // .. (string/array/map concatenation, C4Script AB_Concat)
    ConcatEqual,           // ..= (concat assignment, AB_ConcatIt)
    Question,              // ? (strict-3 safe navigation)
    QuestionQuestion,      // ?? (nil coalescing, AB_NilCoalescing)
    QuestionQuestionEqual, // ??= (nil-coalescing assignment, AB_NilCoalescingIt)
    Equal,
    EqualEqual,
    Bang,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    AndAnd,
    OrOr,
    Pipe,
    Arrow,
    Tilde,
    // Bitwise operators
    Ampersand,
    Caret,
    LeftShift,
    RightShift,
    LBracket,
    RBracket,
    // String comparison operators
    StringEqual, // S=
    // Compound assignment operators
    PlusEqual,
    MinusEqual,
    StarStarEqual,
    StarEqual,
    SlashEqual,
    PercentEqual,
    AndEqual,
    OrEqual,
    XorEqual,
    LeftShiftEqual,
    RightShiftEqual,
}

/// Where C4AulParseError points: `SGetLine` and `SLineGetCharacters` at a read
/// position in the loaded script (C4AulParse.cpp:268-294). That is the count of
/// newlines before the position, and the bytes since the last newline with
/// the newline itself counted (C4Strings.cpp:380-403).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticPosition {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl DiagnosticPosition {
    /// The position after `character`, counted in the C4 string bytes C4Aul
    /// reads.
    pub(crate) fn advanced_past(self, character: char) -> Self {
        if character == '\n' {
            return Self {
                line: self.line + 1,
                column: 1,
            };
        }
        let bytes = crate::value::c4_string_byte_len(character.encode_utf8(&mut [0; 4]));
        self.after_bytes(bytes)
    }

    /// The position `count` bytes further along the same line.
    pub(crate) fn after_bytes(self, count: usize) -> Self {
        Self {
            column: self.column + count,
            ..self
        }
    }
}

impl Token {
    /// C4Aul reports an error about its current token once it has read past
    /// it, so the position is the one after the token.
    pub(crate) fn diagnostic_position(&self) -> DiagnosticPosition {
        self.read_end
    }

    pub(crate) fn read_start(&self) -> DiagnosticPosition {
        self.read_start
    }

    pub(crate) fn with_read_span(self, start: DiagnosticPosition, end: DiagnosticPosition) -> Self {
        Self {
            read_start: start,
            read_end: end,
            ..self
        }
    }

    pub fn new(kind: TokenKind, line: usize, column: usize) -> Self {
        Self {
            kind,
            line,
            column,
            raw_number: None,
            number_is_hex: false,
            read_start: DiagnosticPosition::default(),
            read_end: DiagnosticPosition::default(),
        }
    }

    pub(crate) fn new_number(
        value: i32,
        raw_number: u64,
        number_is_hex: bool,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            raw_number: Some(raw_number),
            number_is_hex,
            ..Self::new(TokenKind::Number(value), line, column)
        }
    }

    pub(crate) fn raw_number(&self) -> Option<u64> {
        self.raw_number
    }

    pub(crate) fn number_is_hex(&self) -> bool {
        self.number_is_hex
    }
}
