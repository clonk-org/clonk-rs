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
    Id { id: String, nowarn: bool },
    Wildcard, // Append to all definitions (*)
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
    /// Whether this is the first entry in its comma-delimited declaration.
    /// Registration recovery skips only the failed group, then resumes at a
    /// later declaration (including one compiled in another script host).
    pub starts_declaration_group: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub functions: Vec<Function>,
    pub var_decls: Vec<VarDecl>, // Script-wide local/static declarations
    /// Held C4String operands in parser encounter order: quoted literals plus
    /// identifier-backed map/property keys. Static-constant strings are
    /// excluded because C4Aul registers those through a referenced GlobalConst
    /// value rather than granting parser Hold.
    pub string_literals: Vec<String>,
    pub includes: Vec<String>, // List of included definition IDs
    /// `#appendto` targets (C++ `C4AulScript::Appends`, a LIST —
    /// C4AulParse.cpp:1485; scripts may carry several).
    pub appends: Vec<AppendTo>,
    pub strict_level: Option<u8>, // Strict mode level (1, 2, or 3)
}

impl Script {
    pub fn with_directives(
        functions: Vec<Function>,
        var_decls: Vec<VarDecl>,
        string_literals: Vec<String>,
        includes: Vec<String>,
        appends: Vec<AppendTo>,
        strict_level: Option<u8>,
    ) -> Self {
        Self {
            functions,
            var_decls,
            string_literals,
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
    Map,
    Any,
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
            TypeAnnotation::Map => f.write_str("map"),
            TypeAnnotation::Any => f.write_str("any"),
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
    /// Script which originally declared this function (`pOrgScript` in
    /// C4Aul). Include/append copies keep this identity even though their
    /// destination owner changes. Bound when the parsed script is installed
    /// in an [`Engine`](crate::Engine).
    pub(crate) source_host: Option<crate::vm::ScriptHostIdentity>,
    /// Human-readable name of the original script host (`pOrgScript`'s
    /// `ScriptName`) used by runtime call-stack diagnostics.
    pub(crate) source_name: Option<String>,
    /// Zero-based source line of the function name, matching
    /// `C4AulScriptFunc::SGetLine`.
    pub(crate) source_line: usize,
    /// The script host referenced by a global function's C4Aul `LinkedTo`
    /// pointer. Global functions execute from the shared engine table, but
    /// native local-function lookup still starts in their declaring host.
    /// Non-global functions always use the destination VM host instead.
    pub(crate) global_link_host: Option<crate::vm::ScriptHostIdentity>,
    /// The function this one overloaded (C++ `Fn->OwnerOverloaded`): a later
    /// script redefining the name, or an #include'd parent's same-name
    /// function. `inherited(...)`/`_inherited(...)` call it.
    pub overloaded: Option<std::sync::Arc<Function>>,
    /// One-based source line of a hard `inherited(...)` call in this body, if
    /// it has one. C4Aul binds `inherited` while parsing function bodies —
    /// which happens after every func table is built — and refuses the
    /// function outright when `Fn->OwnerOverloaded` is null
    /// (`C4AulParse.cpp:2799`). The port parses bodies before linking, so it
    /// records the site here and runs the same check once the overload tables
    /// exist. The failsafe `_inherited` spelling never sets it.
    pub(crate) hard_inherited_line: Option<usize>,
    /// Lazily lowered local-only instruction stream. This is derived state:
    /// function equality remains solely a property of the parsed script.
    pub(crate) compiled: std::sync::OnceLock<crate::vm::CompiledFunctionCache>,
    /// Immutable C4AulFunc-style pointer retained by native callback queues.
    /// Resolution reuses this snapshot until a link mutation replaces it, so
    /// its lazily compiled body is shared across repeated callbacks too.
    pub(crate) resolved_snapshot: std::sync::OnceLock<std::sync::Arc<Function>>,
}

impl std::fmt::Debug for Function {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Function")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("body", &self.body)
            .field("access", &self.access)
            .field("returns_reference", &self.returns_reference)
            .field("description", &self.description)
            .field("strict_level", &self.strict_level)
            .field("source_host", &self.source_host)
            .field("source_name", &self.source_name)
            .field("source_line", &self.source_line)
            .field("global_link_host", &self.global_link_host)
            .field("overloaded", &self.overloaded)
            .field("hard_inherited_line", &self.hard_inherited_line)
            .finish()
    }
}

impl Clone for Function {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            params: self.params.clone(),
            body: self.body.clone(),
            access: self.access,
            returns_reference: self.returns_reference,
            description: self.description.clone(),
            strict_level: self.strict_level,
            source_host: self.source_host,
            source_name: self.source_name.clone(),
            source_line: self.source_line,
            global_link_host: self.global_link_host,
            overloaded: self.overloaded.clone(),
            hard_inherited_line: self.hard_inherited_line,
            compiled: std::sync::OnceLock::new(),
            resolved_snapshot: std::sync::OnceLock::new(),
        }
    }
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.params == other.params
            && self.body == other.body
            && self.access == other.access
            && self.returns_reference == other.returns_reference
            && self.description == other.description
            && self.strict_level == other.strict_level
            && self.source_host == other.source_host
            && self.source_name == other.source_name
            && self.source_line == other.source_line
            && self.global_link_host == other.global_link_host
            && self.overloaded == other.overloaded
            && self.hard_inherited_line == other.hard_inherited_line
    }
}

impl Function {
    pub(crate) fn reset_compiled_cache(&mut self) {
        self.compiled = std::sync::OnceLock::new();
        self.resolved_snapshot = std::sync::OnceLock::new();
    }

    pub(crate) fn resolved_snapshot(&self) -> std::sync::Arc<Self> {
        std::sync::Arc::clone(
            self.resolved_snapshot
                .get_or_init(|| std::sync::Arc::new(self.clone())),
        )
    }

    fn reset_resolved_snapshot_chain(&mut self) {
        self.resolved_snapshot = std::sync::OnceLock::new();
        if let Some(overloaded) = self.overloaded.as_mut() {
            std::sync::Arc::make_mut(overloaded).reset_resolved_snapshot_chain();
        }
    }

    /// First named local candidate in this overload chain. Rust keeps
    /// declarations under their source name, while C4Aul represents every
    /// global declaration with an unnamed link in that source host; skip
    /// those global nodes without hiding an older ordinary same-name func.
    pub(crate) fn first_non_global(&self) -> Option<&Self> {
        std::iter::successors(Some(self), |function| function.overloaded.as_deref())
            .find(|function| function.access != AccessLevel::Global)
    }

    /// The function this one overloaded, resolved in its OWNER's function
    /// list like `Fn->OwnerOverloaded = Fn->Owner->GetOverloadedFunc(Fn)`
    /// (C4AulParse.cpp:1406-1408, whose comment reads "*MUST* check
    /// Fn->Owner-list, because it may be the engine (due to linked globals)").
    ///
    /// A `global func` is built as `new C4AulScriptFunc(a->Engine, Idtf)` and
    /// leaves only an UNNAMED `C4AulFunc(a, nullptr)` link in the declaring
    /// script (C4AulParse.cpp:1608-1615), so:
    ///
    /// * an engine-owned function walks the ENGINE's list, which holds only
    ///   natives and other globals — never a definition-scope function of the
    ///   declaring host;
    /// * a definition-scope function walks its own host's list first, where
    ///   the unnamed link never matches `SEqual(ByFunc->Name, f->Name)`
    ///   (C4Aul.cpp:269-276), and only when that finds nothing does it hop to
    ///   the engine (`if (!f && Owner)`, C4Aul.cpp:281-288).
    ///
    /// Rust keys one overload chain per name for the whole host, so both cuts
    /// are applied here rather than at chain construction: the chain is also
    /// how `first_non_global` keeps the host's own declaration reachable.
    pub(crate) fn owner_overloaded(&self) -> Option<&std::sync::Arc<Self>> {
        let ancestors = || {
            std::iter::successors(self.overloaded.as_ref(), |parent| {
                parent.overloaded.as_ref()
            })
        };
        let engine_owned =
            |function: &&std::sync::Arc<Self>| function.access == AccessLevel::Global;
        if self.access == AccessLevel::Global {
            ancestors().find(engine_owned)
        } else {
            ancestors()
                .find(|function| !engine_owned(function))
                .or_else(|| ancestors().find(engine_owned))
        }
    }

    /// Hang `parent` at the tail of this function's overload chain (C++
    /// `Fn->OwnerOverloaded`). Idempotent for repeat-link callers: a parent
    /// already on the chain is replaced when it has gained its own chain.
    pub fn push_overload(&mut self, parent: Function) {
        self.reset_resolved_snapshot_chain();
        fn same_definition(a: &Function, b: &Function) -> bool {
            a.name == b.name
                && a.params == b.params
                && a.body == b.body
                && a.access == b.access
                && a.returns_reference == b.returns_reference
                && a.description == b.description
                && a.strict_level == b.strict_level
                && a.source_host == b.source_host
                && a.source_name == b.source_name
                && a.source_line == b.source_line
                && a.global_link_host == b.global_link_host
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
        self.reset_resolved_snapshot_chain();
        let mut tail = &mut self.overloaded;
        while let Some(next) = tail {
            tail = &mut std::sync::Arc::make_mut(next).overloaded;
        }
        *tail = Some(std::sync::Arc::new(parent));
    }

    /// Stamp every global node in this declaration's overload chain with
    /// its declaring script host. `Arc::make_mut` preserves the provenance
    /// when a parsed same-name chain is shared.
    pub(crate) fn bind_global_link_host(&mut self, host: crate::vm::ScriptHostIdentity) {
        self.resolved_snapshot = std::sync::OnceLock::new();
        if self.access == AccessLevel::Global {
            self.global_link_host = Some(host);
        }
        if let Some(overloaded) = self.overloaded.as_mut() {
            std::sync::Arc::make_mut(overloaded).bind_global_link_host(host);
        }
    }

    /// Stamp the original declaring script on parsed functions. Linked
    /// copies already carrying provenance deliberately retain it.
    pub(crate) fn bind_source_host(&mut self, host: crate::vm::ScriptHostIdentity) {
        self.resolved_snapshot = std::sync::OnceLock::new();
        if self.source_host.is_none() {
            self.source_host = Some(host);
        }
        if let Some(overloaded) = self.overloaded.as_mut() {
            std::sync::Arc::make_mut(overloaded).bind_source_host(host);
        }
    }

    /// Stamp the original script's diagnostic name. Linked copies retain the
    /// name already assigned by their declaring host.
    pub(crate) fn bind_source_name(&mut self, name: &str) {
        self.resolved_snapshot = std::sync::OnceLock::new();
        if self.source_name.is_none() {
            self.source_name = Some(name.to_owned());
        }
        if let Some(overloaded) = self.overloaded.as_mut() {
            std::sync::Arc::make_mut(overloaded).bind_source_name(name);
        }
    }

    /// Rename diagnostics for functions originating in one script host.
    /// Include/append copies retain their declaring host's ScriptName.
    pub(crate) fn rebind_source_name_for_host(
        &mut self,
        host: crate::vm::ScriptHostIdentity,
        name: &str,
    ) {
        self.resolved_snapshot = std::sync::OnceLock::new();
        if self.source_host == Some(host) {
            self.source_name = Some(name.to_owned());
        }
        if let Some(overloaded) = self.overloaded.as_mut() {
            std::sync::Arc::make_mut(overloaded).rebind_source_name_for_host(host, name);
        }
    }

    /// Original script host (`pOrgScript`) for diagnostic/source lookup.
    pub fn source_host_identity(&self) -> Option<crate::vm::ScriptHostIdentity> {
        self.source_host
    }

    pub fn source_name(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    /// Zero-based declaration line, matching C4Aul's `SGetLine` output.
    pub fn source_line(&self) -> usize {
        self.source_line
    }

    /// Whether this declaration lives in the engine-global function table.
    pub fn is_global(&self) -> bool {
        self.access == AccessLevel::Global
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
    /// A statement whose first token is the bare call `goto(...)`. C++ emits
    /// AB_RETURN immediately after that call for NONSTRICT origin scripts,
    /// before compiling any remaining expression suffix. Strict scripts
    /// execute the complete expression normally.
    LegacyGoto {
        call: Expr,
        expression: Expr,
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
    Index(Box<AssignmentTarget>, IndexOperand), // arr[index] as lvalue
    /// `expression[]`: AB_ARRAY_APPEND operates on the current stack value.
    /// A referenced array yields its new last-slot reference; a self-owned
    /// temporary loses that reference with its container and collapses to nil.
    ArrayAppend(Box<Expr>),
    LocalSlot(Box<Expr>),  // Local(expr) as lvalue - object-local slot
    VarSlot(Box<Expr>),    // Var(expr) as lvalue - function-local slot
    EffectSlot(Vec<Expr>), // EffectVar(index, target, effect_num) as lvalue - effect variable slot
    MethodSlot {
        // obj->LocalN("key") as lvalue - method-accessed slot
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        /// Distinguishes `obj->Fn(arg)` (target + ten slots) from the
        /// normalized direct native spelling `Fn(arg, obj)` (native arity).
        is_arrow: bool,
    },
    FunctionCall {
        // func(&...) as lvalue - reference-returning function call
        name: String,
        args: Vec<Expr>,
    },
    GlobalFunctionCall {
        // global->func(&...) as lvalue - exact engine reference return
        name: String,
        args: Vec<Expr>,
        failsafe: bool,
        forward_rest: bool,
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
    /// NONSTRICT/STRICT1 `if` and `while` headers use `Parse_Params(1)`.
    /// Every explicit slot is evaluated before all but the first are dropped;
    /// `...` supplies the first unnamed caller parameter when no explicit
    /// slot already fills the one-value frame.
    LegacyParameterList {
        args: Vec<Expr>,
        forward_rest: bool,
    },
    /// Strict-3 `global->Fn(...)` / `global->~Fn(...)`: engine-owner lookup
    /// with no object or definition context (C4Aul AB_CALLGLOBAL).
    GlobalCall {
        name: String,
        args: Vec<Expr>,
        failsafe: bool,
        forward_rest: bool,
    },
    Array(Vec<Expr>),
    Proplist(Vec<(Expr, Expr)>),
    Index(Box<Expr>, IndexOperand),
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
    /// Compound assignments retain one evaluated lvalue reference across
    /// their read, operation, and write. Desugaring to `a = a op b` would
    /// evaluate side-effecting target expressions twice.
    CompoundAssignment {
        target: AssignmentTarget,
        operation: BinaryOp,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct SafeNavigationStep {
    pub nil_guard: bool,
    pub operation: NavigationOperation,
}

/// C4Aul emits a dedicated AB_MAPA_R/V operand for a direct string token in
/// `value["key"]`. An equivalent expression such as `value[("key")]` uses
/// an ordinary value-stack slot instead.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexOperand {
    EmbeddedString(String),
    Dynamic(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavigationOperation {
    Index(IndexOperand),
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
    KeywordStringEqual,
    KeywordStringNotEqual,
}
