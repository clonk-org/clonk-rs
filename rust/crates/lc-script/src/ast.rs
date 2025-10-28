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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarDeclKind {
    Local,       // local x; - per-instance field
    Static,      // static x; - definition-shared storage
    StaticConst, // static const x = ...; - immutable constant
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub kind: VarDeclKind,
    pub name: String,
    pub init: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub functions: Vec<Function>,
    pub var_decls: Vec<VarDecl>,       // Top-level variable declarations
    pub includes: Vec<String>,         // List of included definition IDs
    pub appendto: Option<AppendTo>,    // Optional append target
    pub strict_level: Option<u8>,      // Strict mode level (1, 2, or 3)
}

impl Script {
    pub fn new(functions: Vec<Function>) -> Self {
        Self {
            functions,
            var_decls: Vec::new(),
            includes: Vec::new(),
            appendto: None,
            strict_level: None,
        }
    }

    pub fn with_directives(
        functions: Vec<Function>,
        var_decls: Vec<VarDecl>,
        includes: Vec<String>,
        appendto: Option<AppendTo>,
        strict_level: Option<u8>,
    ) -> Self {
        Self {
            functions,
            var_decls,
            includes,
            appendto,
            strict_level,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    Int,
    Bool,
    String,
    Object,
    Id,
    Array,
    Proplist,
    Effect,
    Nil,
    Union(Vec<TypeAnnotation>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
}

impl Parameter {
    pub fn new(name: String) -> Self {
        Self {
            name,
            type_annotation: None,
        }
    }

    pub fn with_type(name: String, type_annotation: TypeAnnotation) -> Self {
        Self {
            name,
            type_annotation: Some(type_annotation),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Parameter>,
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
    ForIn {
        variable: String,      // The iteration variable
        declare_var: bool,     // true if using "var variable", false if pre-declared
        iterable: Expr,        // Expression to iterate over
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
    Index(Box<AssignmentTarget>, Box<Expr>), // arr[index] as lvalue
    LocalSlot(Box<Expr>), // Local(expr) as lvalue - object-local slot
    VarSlot(Box<Expr>),   // Var(expr) as lvalue - function-local slot
    EffectSlot(Vec<Expr>), // EffectVar(index, target, effect_num) as lvalue - effect variable slot
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String),
    This,
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    Call { callee: Box<Expr>, args: Vec<Expr>, is_optional: bool, forward_rest: bool },
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
    Negate,     // - (arithmetic negation)
    Not,        // ! (logical NOT)
    BitwiseNot, // ~ (bitwise NOT)
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
    // String comparison operators
    StringEqual,
    StringNotEqual,
    StringLess,
    StringLessEqual,
    StringGreater,
    StringGreaterEqual,
}
