use std::fmt;

use crate::value::Literal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessLevel {
    #[default]
    Public, // Default, accessible from anywhere
    Protected, // Accessible within definition and derived definitions
    Private,   // Only accessible within the definition
    Global,    // Global scope, accessible across all definitions
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppendTo {
    Id(String), // Append to specific definition
    Wildcard,   // Append to all definitions (*)
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
    pub var_decls: Vec<VarDecl>,    // Top-level variable declarations
    pub includes: Vec<String>,      // List of included definition IDs
    /// `#appendto` targets (C++ `C4AulScript::Appends`, a LIST —
    /// C4AulParse.cpp:1485; scripts may carry several).
    pub appends: Vec<AppendTo>,
    pub strict_level: Option<u8>,   // Strict mode level (1, 2, or 3)
}

impl Script {
    #[allow(dead_code)]
    pub fn new(functions: Vec<Function>) -> Self {
        Self {
            functions,
            var_decls: Vec::new(),
            includes: Vec::new(),
            appends: Vec::new(),
            strict_level: None,
        }
    }

    pub fn with_directives(
        functions: Vec<Function>,
        var_decls: Vec<VarDecl>,
        includes: Vec<String>,
        appends: Vec<AppendTo>,
        strict_level: Option<u8>,
    ) -> Self {
        Self {
            functions,
            var_decls,
            includes,
            appends,
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
    Any,
    Union(Vec<TypeAnnotation>),
}

impl fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeAnnotation::Int => f.write_str("int"),
            TypeAnnotation::Bool => f.write_str("bool"),
            TypeAnnotation::String => f.write_str("string"),
            TypeAnnotation::Object => f.write_str("object"),
            TypeAnnotation::Id => f.write_str("id"),
            TypeAnnotation::Array => f.write_str("array"),
            TypeAnnotation::Proplist => f.write_str("proplist"),
            TypeAnnotation::Effect => f.write_str("effect"),
            TypeAnnotation::Nil => f.write_str("nil"),
            TypeAnnotation::Any => f.write_str("any"),
            TypeAnnotation::Union(types) => {
                let joined = types
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("|");
                f.write_str(&joined)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub is_reference: bool,
}

impl Parameter {
    pub fn new(name: String) -> Self {
        Self {
            name,
            type_annotation: None,
            is_reference: false,
        }
    }

    pub fn with_type(name: String, type_annotation: TypeAnnotation) -> Self {
        Self {
            name,
            type_annotation: Some(type_annotation),
            is_reference: false,
        }
    }

    pub fn with_reference(
        name: String,
        type_annotation: Option<TypeAnnotation>,
        is_reference: bool,
    ) -> Self {
        Self {
            name,
            type_annotation,
            is_reference,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Parameter>,
    pub body: Vec<Stmt>,
    pub access: AccessLevel,
    pub returns_reference: bool,
    /// Raw localized function-description metadata from the leading
    /// `[caption|Image=...|Condition=...]` block. C4Aul retains this on the
    /// script function for context-menu discovery (C4AulParse.cpp:1825-1853;
    /// C4ObjectMenu.cpp:670-682).
    pub description: Option<String>,
    /// The `#strict` level of the script this function came from (C++ uses the
    /// owning script's strict level for `==`/`!=`, `Fn->pOrgScript->Strict`).
    /// `None` = no `#strict` directive (NONSTRICT). Stamped in `Script::from_ast`.
    pub strict_level: Option<u8>,
    /// The function this one overloaded (C++ `Fn->OwnerOverloaded`): a later
    /// script redefining the name, or an #include'd parent's same-name
    /// function. `inherited(...)`/`_inherited(...)` call it.
    pub overloaded: Option<std::sync::Arc<Function>>,
}

impl Function {
    /// Hang `parent` at the tail of this function's overload chain (C++
    /// `Fn->OwnerOverloaded`). Idempotent for repeat-link callers: a parent
    /// already on the chain is replaced when it has gained its own chain.
    pub fn push_overload(&mut self, parent: Function) {
        fn same_definition(a: &Function, b: &Function) -> bool {
            a.name == b.name
                && a.params == b.params
                && a.body == b.body
                && a.access == b.access
                && a.returns_reference == b.returns_reference
                && a.description == b.description
                && a.strict_level == b.strict_level
        }
        let mut tail = &mut self.overloaded;
        loop {
            let found = tail
                .as_deref()
                .is_some_and(|next| same_definition(next, &parent));
            if found {
                if parent.overloaded.is_some() {
                    *tail = Some(std::sync::Arc::new(parent));
                }
                return;
            }
            match tail {
                Some(next) => tail = &mut std::sync::Arc::make_mut(next).overloaded,
                None => {
                    *tail = Some(std::sync::Arc::new(parent));
                    return;
                }
            }
        }
    }

    /// Append an include copy without structural deduplication. C++ creates a
    /// distinct C4AulScriptFunc for every include edge, including identical
    /// bodies and diamond paths (C4AulLink.cpp:113-141).
    pub fn append_include_overload(&mut self, parent: Function) {
        let mut tail = &mut self.overloaded;
        while let Some(next) = tail {
            tail = &mut std::sync::Arc::make_mut(next).overloaded;
        }
        *tail = Some(std::sync::Arc::new(parent));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// C4Aul's AB_ERR bytecode sentinel: a function body parse failure does
    /// not remove the function symbol, but raises when execution reaches the
    /// broken suffix (C4AulParse.cpp:3549-3577).
    ParseError {
        message: String,
        line: usize,
        column: usize,
    },
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
        variable: String, // The array item or map key variable
        /// Present for map foreach (`for (key, value in map)`); absent for
        /// array foreach (`for (value in array)`).
        value_variable: Option<String>,
        declare_var: bool, // true if using "var variable", false if pre-declared
        iterable: Expr,    // Expression to iterate over
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),
    // Sequence executes statements sequentially WITHOUT creating a new scope
    // Used for multi-variable declarations: var a, b, c;
    Sequence(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    VarDecls(Vec<(String, Option<Expr>)>), // var i = 0, j = 1
    Expr(Expr),                            // i = 0
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignmentTarget {
    /// A value expression accepted syntactically by C4Aul's precedence
    /// parser but rejected by AB_Set's runtime reference conversion.
    InvalidValue {
        expression: Box<Expr>,
        operator: &'static str,
    },
    Variable(String),
    Property(Box<AssignmentTarget>, String),
    Index(Box<AssignmentTarget>, Box<Expr>), // arr[index] as lvalue
    /// `array[]`: AB_ARRAY_APPEND grows the array immediately and yields a
    /// reference to the new last slot.
    ArrayAppend(Box<AssignmentTarget>),
    LocalSlot(Box<Expr>),                    // Local(expr) as lvalue - object-local slot
    VarSlot(Box<Expr>),                      // Var(expr) as lvalue - function-local slot
    EffectSlot(Vec<Expr>), // EffectVar(index, target, effect_num) as lvalue - effect variable slot
    MethodSlot {
        // obj->LocalN("key") as lvalue - method-accessed slot
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    FunctionCall {
        // func(&...) as lvalue - reference-returning function call
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String),
    This,
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        is_optional: bool,
        forward_rest: bool,
    },
    Array(Vec<Expr>),
    Proplist(Vec<(Expr, Expr)>),
    Index(Box<Expr>, Box<Expr>),
    Property(Box<Expr>, String),
    ArrayAppend(Box<Expr>),
    /// Assignment to `array[]` must retain the new append reference while the
    /// RHS runs. Compound assignment also must not evaluate the append
    /// expression a second time through the generic desugaring.
    ArrayAppendAssignment {
        target: AssignmentTarget,
        operation: Option<BinaryOp>,
        operator: &'static str,
        value: Box<Expr>,
    },
    /// Strict-3 `receiver?->Call()`, `receiver?[index]`, and
    /// `receiver?.property`. The first step and every step preceded by a
    /// later `?` guards the complete remaining suffix on nil; the node is
    /// always an rvalue (C4AulParse.cpp:3105-3129).
    SafeNavigation {
        receiver: Box<Expr>,
        steps: Vec<SafeNavigationStep>,
    },
    // Increment/Decrement (require lvalue)
    PreIncrement(Box<Expr>),
    PreDecrement(Box<Expr>),
    PostIncrement(Box<Expr>),
    PostDecrement(Box<Expr>),
    // Assignment as an expression (right-associative)
    Assignment(AssignmentTarget, Box<Expr>),
    // Comma operator - sequence of expressions, evaluates all and returns last (lowest precedence)
    Comma(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SafeNavigationStep {
    pub nil_guard: bool,
    pub operation: NavigationOperation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavigationOperation {
    Index(Box<Expr>),
    ArrayAppend,
    Property(String),
    MethodCall {
        name: String,
        args: Vec<Expr>,
        is_optional: bool,
        forward_rest: bool,
    },
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
    Concat, // .. (string/array/map concatenation, C4Script AB_Concat)
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    /// `??` — nil coalescing (AB_NilCoalescing, C4AulParse.cpp:464): the
    /// right side runs only when the left is NIL (0/false are kept).
    NilCoalescing,
    // Bitwise operators
    BitAnd,
    BitOr,
    BitXor,
    LeftShift,
    RightShift,
    // String comparison operators
    StringEqual,
    StringNotEqual,
    KeywordStringEqual,
    KeywordStringNotEqual,
    StringLess,
    StringLessEqual,
    StringGreater,
    StringGreaterEqual,
}
