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

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Func,
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
    // Keyword operators (synonyms for symbolic operators)
    Eq,  // ==
    Ne,  // !=
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
    And, // &&
    Or,  // ||
    Not, // !
}

impl Keyword {
    /// The source text of the keyword. C4Aul keywords are contextual (the
    /// C++ tokenizer emits plain ATT_IDTF words), so parse positions that
    /// accept arbitrary identifiers — parameter names, expression variables
    /// — fall back to this lexeme.
    pub fn lexeme(self) -> &'static str {
        match self {
            Keyword::Func => "func",
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
            Keyword::Eq => "eq",
            Keyword::Ne => "ne",
            Keyword::Lt => "lt",
            Keyword::Le => "le",
            Keyword::Gt => "gt",
            Keyword::Ge => "ge",
            Keyword::And => "and",
            Keyword::Or => "or",
            Keyword::Not => "not",
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

impl Token {
    pub fn new(kind: TokenKind, line: usize, column: usize) -> Self {
        Self {
            kind,
            line,
            column,
            raw_number: None,
            number_is_hex: false,
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
            kind: TokenKind::Number(value),
            line,
            column,
            raw_number: Some(raw_number),
            number_is_hex,
        }
    }

    pub(crate) fn raw_number(&self) -> Option<u64> {
        self.raw_number
    }

    pub(crate) fn number_is_hex(&self) -> bool {
        self.number_is_hex
    }
}
