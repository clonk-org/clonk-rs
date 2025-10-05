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
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Func,
    Global,
    Var,
    If,
    Else,
    While,
    Return,
    True,
    False,
    Nil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
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
}

impl Token {
    pub fn new(kind: TokenKind, line: usize, column: usize) -> Self {
        Self { kind, line, column }
    }
}
