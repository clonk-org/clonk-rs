use crate::value::Literal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    Public,    // Default, accessible from anywhere
    Protected, // Accessible within definition and derived definitions
    Private,   // Only accessible within the definition
    Global,    // Global scope, accessible across all definitions
}

impl Default for AccessLevel {
    fn default() -> Self {
        AccessLevel::Public
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppendTo {
    Id(String),      // Append to specific definition
    Wildcard,        // Append to all definitions (*)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub functions: Vec<Function>,
    pub includes: Vec<String>,         // List of included definition IDs
    pub appendto: Option<AppendTo>,    // Optional append target
    pub strict_level: Option<u8>,      // Strict mode level (1, 2, or 3)
}

impl Script {
    pub fn new(functions: Vec<Function>) -> Self {
        Self {
            functions,
            includes: Vec::new(),
            appendto: None,
            strict_level: None,
        }
    }

    pub fn with_directives(
        functions: Vec<Function>,
        includes: Vec<String>,
        appendto: Option<AppendTo>,
        strict_level: Option<u8>,
    ) -> Self {
        Self {
            functions,
            includes,
            appendto,
            strict_level,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub access: AccessLevel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    VarDecl {
        name: String,
        init: Option<Expr>,
    },
    Assignment {
        target: AssignmentTarget,
        value: Expr,
    },
    Return(Option<Expr>),
    Break,
    Continue,
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
    For {
        init: Option<ForInit>,
        condition: Option<Expr>,
        increment: Option<Expr>,
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    VarDecls(Vec<(String, Option<Expr>)>), // var i = 0, j = 1
    Expr(Expr),                             // i = 0
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignmentTarget {
    Variable(String),
    Property(Box<AssignmentTarget>, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    Call { callee: Box<Expr>, args: Vec<Expr>, is_optional: bool },
    Array(Vec<Expr>),
    Proplist(Vec<(String, Expr)>),
    Index(Box<Expr>, Box<Expr>),
    Property(Box<Expr>, String),
    // Increment/Decrement (require lvalue)
    PreIncrement(Box<Expr>),
    PreDecrement(Box<Expr>),
    PostIncrement(Box<Expr>),
    PostDecrement(Box<Expr>),
    // Assignment as an expression (lowest precedence, right-associative)
    Assignment(AssignmentTarget, Box<Expr>),
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
    // Bitwise operators
    BitAnd,
    BitOr,
    BitXor,
    LeftShift,
    RightShift,
}
