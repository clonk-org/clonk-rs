use crate::value::Literal;

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub functions: Vec<Function>,
}

impl Script {
    pub fn new(functions: Vec<Function>) -> Self {
        Self { functions }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub is_global: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    VarDecl {
        name: String,
        init: Option<Expr>,
    },
    Assignment {
        name: String,
        value: Expr,
    },
    Return(Option<Expr>),
    Expr(Expr),
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    Call { callee: String, args: Vec<Expr> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}
