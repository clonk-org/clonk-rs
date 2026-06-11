#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number(i32),
    String(String),
    C4Id(String), // 4-character definition ID like "CLNK", "COWB"
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
    // Type keywords
    Int,
    Bool,
    String,
    Object,
    Id,
    Array,
    Proplist,
    Effect,
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
            Keyword::Int => "int",
            Keyword::Bool => "bool",
            Keyword::String => "string",
            Keyword::Object => "object",
            Keyword::Id => "id",
            Keyword::Array => "array",
            Keyword::Proplist => "proplist",
            Keyword::Effect => "effect",
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
    Ellipsis,    // ... (varargs forwarder)
    Concat,      // .. (string/array/map concatenation, C4Script AB_Concat)
    ConcatEqual, // ..= (concat assignment, AB_ConcatIt)
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
    StringEqual,        // S=
    StringNotEqual,     // S!=
    StringLess,         // S<
    StringLessEqual,    // S<=
    StringGreater,      // S>
    StringGreaterEqual, // S>=
    // Compound assignment operators
    PlusEqual,
    MinusEqual,
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
        Self { kind, line, column }
    }
}
