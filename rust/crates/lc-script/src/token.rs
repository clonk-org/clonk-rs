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
    Keyword(Keyword),
    Symbol(Symbol),
    Directive(String), // Directive like "#include", "#appendto", "#strict"
    LocaleKey(String), // Localization key like $TxtPermanentModeTurnOn$
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Func,
    Global,
    Private,
    Protected,
    Public,
    Local,
    Var,
    If,
    Else,
    While,
    For,
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
    Eq,   // ==
    Ne,   // !=
    Lt,   // <
    Le,   // <=
    Gt,   // >
    Ge,   // >=
    And,  // &&
    Or,   // ||
    Not,  // !
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
    Plus,
    PlusPlus,
    Minus,
    MinusMinus,
    Star,
    Slash,
    Percent,
    Dot,
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
