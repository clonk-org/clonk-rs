use crate::ast::{
    AccessLevel, AppendTo, AssignmentTarget, BinaryOp, Expr, Function, Parameter, Script, Stmt,
    TypeAnnotation, UnaryOp, VarDecl, VarDeclKind,
};
use crate::error::ParseError;
use crate::lexer::Lexer;
use crate::token::{Keyword, Symbol, Token, TokenKind};
use crate::value::Literal;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    // Additional buffer for multi-token lookahead (used in for-loop disambiguation)
    lookahead_buffer: Vec<Token>,
    // Track consumed tokens during speculative parsing
    speculative_tokens: Option<Vec<Token>>,
    // C4Aul's current per-script strictness. Legacy `Name:` declarations are
    // legal only below STRICT2 (C4AulParse.cpp:1715-1717).
    strict_level: u8,
    /// Logical brace depth of consumed tokens. Recovery uses this to skip a
    /// broken function's remaining body without swallowing the next
    /// top-level declaration.
    brace_depth: usize,
    /// Logical stream progress, excluding speculative tokens that were put
    /// back. C4Aul advances past an offending top-level token only when the
    /// failed parse attempt itself made no progress.
    consumed_tokens: usize,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            lexer: Lexer::new(source),
            peeked: None,
            lookahead_buffer: Vec::new(),
            speculative_tokens: None,
            strict_level: 0,
            brace_depth: 0,
            consumed_tokens: 0,
        }
    }

    /// DirectExec parsing (C4AulScript::ParseFn fExprOnly,
    /// C4AulParse.cpp:1417-1424): the source is ONE expression; anything
    /// after it — e.g. content's stray trailing `;` — is ignored.
    pub fn parse_direct_exec_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_expression()
    }

    #[allow(dead_code)]
    pub fn parse_script(&mut self) -> Result<Script, ParseError> {
        // Parse directives, variable declarations, and functions
        // Directives and variable declarations can be interspersed
        let mut includes = Vec::new();
        let mut appends = Vec::new();
        let mut strict_level = None;
        let mut var_decls = Vec::new();
        let mut functions = Vec::new();

        while !self.is_eof()? {
            // Check for directives
            if let Some(directive) = self.try_parse_directive()? {
                match directive.as_str() {
                    "#include" => {
                        let id = self.next()?;
                        match id.kind {
                            TokenKind::Identifier(id_str) | TokenKind::C4Id(id_str) => {
                                includes.push(id_str);
                            }
                            _ => {
                                return Err(ParseError::new(
                                    "expected definition ID after #include",
                                    id.line,
                                    id.column,
                                ))
                            }
                        }
                    }
                    "#appendto" => {
                        let next = self.next()?;
                        appends.push(match &next.kind {
                            TokenKind::Identifier(id) | TokenKind::C4Id(id) => {
                                AppendTo::Id(id.clone())
                            }
                            TokenKind::Symbol(Symbol::Star) => AppendTo::Wildcard,
                            _ => {
                                return Err(ParseError::new(
                                    "expected definition ID or '*' after #appendto",
                                    next.line,
                                    next.column,
                                ))
                            }
                        });
                        // Optional `nowarn` suffix (C4AUL_NoWarn,
                        // C4AulParse.cpp:1463-1472) suppresses the
                        // missing-target warning; parse-wise just consume.
                        if let Ok(token) = self.peek() {
                            if matches!(&token.kind, TokenKind::Identifier(word) if word == "nowarn")
                            {
                                self.next()?;
                            }
                        }
                    }
                    "#strict" => {
                        // Default to level 1
                        let mut level = 1;
                        // Check if there's a number following
                        if let Ok(token) = self.peek() {
                            if let TokenKind::Number(n) = token.kind {
                                if (1..=3).contains(&n) {
                                    level = n as u8;
                                    self.next()?; // consume the number
                                }
                            }
                        }
                        strict_level = Some(level);
                        self.strict_level = level;
                    }
                    _ => {
                        // Unknown directive, skip it
                    }
                }
            } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Local) {
                // Parse local variable declarations
                self.consume()?; // consume 'local'
                var_decls.extend(self.parse_var_decl_list(VarDeclKind::Local)?);
            } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Static) {
                // Parse static variable declarations
                self.consume()?; // consume 'static'
                                 // Check for 'const' after 'static'
                if self.consume_if_keyword(Keyword::Const)?.is_some() {
                    var_decls.extend(self.parse_var_decl_list(VarDeclKind::StaticConst)?);
                } else {
                    var_decls.extend(self.parse_var_decl_list(VarDeclKind::Static)?);
                }
            } else {
                // Must be a function
                functions.push(self.parse_function()?);
            }
        }

        Ok(Script::with_directives(
            functions,
            var_decls,
            includes,
            appends,
            strict_level,
        ))
    }

    /// C4Aul preparses each top-level declaration independently and later
    /// compiles each function independently (C4AulParse.cpp:1434-1561,
    /// 3549-3577). Return the partial script plus diagnostics instead of
    /// dropping everything after the first error.
    pub fn parse_script_recovering(&mut self) -> (Script, Vec<ParseError>) {
        let mut includes = Vec::new();
        let mut appends = Vec::new();
        let mut strict_level = None;
        let mut var_decls = Vec::new();
        let mut functions = Vec::new();
        let mut diagnostics = Vec::new();
        let mut top_level_ok = true;

        loop {
            match self.is_eof() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    if top_level_ok {
                        diagnostics.push(error);
                    }
                    top_level_ok = false;
                    // Lexer errors consume their offending bytes. Retry at
                    // the next token rather than discarding another one.
                    continue;
                }
            }

            let declaration_start = self.consumed_tokens;
            let declaration_token = match self.peek() {
                Ok(token) => token.clone(),
                Err(error) => {
                    if top_level_ok {
                        diagnostics.push(error);
                    }
                    top_level_ok = false;
                    continue;
                }
            };

            let attempt = (|| -> Result<(), ParseError> {
                if let Some(directive) = self.try_parse_directive()? {
                    match directive.as_str() {
                        "#include" => {
                            let id = self.next()?;
                            match id.kind {
                                TokenKind::Identifier(id_str) | TokenKind::C4Id(id_str) => {
                                    includes.push(id_str);
                                }
                                _ => {
                                    return Err(ParseError::new(
                                        "expected definition ID after #include",
                                        id.line,
                                        id.column,
                                    ))
                                }
                            }
                        }
                        "#appendto" => {
                            let next = self.next()?;
                            appends.push(match &next.kind {
                                TokenKind::Identifier(id) | TokenKind::C4Id(id) => {
                                    AppendTo::Id(id.clone())
                                }
                                TokenKind::Symbol(Symbol::Star) => AppendTo::Wildcard,
                                _ => {
                                    return Err(ParseError::new(
                                        "expected definition ID or '*' after #appendto",
                                        next.line,
                                        next.column,
                                    ))
                                }
                            });
                            if let Ok(token) = self.peek() {
                                if matches!(&token.kind, TokenKind::Identifier(word) if word == "nowarn")
                                {
                                    self.next()?;
                                }
                            }
                        }
                        "#strict" => {
                            // C++ stores STRICT1 before validating an
                            // explicit level, so `#strict 4` retains level 1.
                            strict_level = Some(1);
                            self.strict_level = 1;
                            if let Ok(token) = self.peek().cloned() {
                                if let TokenKind::Number(level) = token.kind {
                                    if (1..=3).contains(&level) {
                                        strict_level = Some(level as u8);
                                        self.strict_level = level as u8;
                                        self.next()?;
                                    } else {
                                        return Err(ParseError::new(
                                            "unknown strict level",
                                            token.line,
                                            token.column,
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            return Err(ParseError::new(
                                format!("unknown directive: {directive}"),
                                declaration_token.line,
                                declaration_token.column,
                            ));
                        }
                    }
                } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Local) {
                    self.consume()?;
                    var_decls.extend(self.parse_var_decl_list(VarDeclKind::Local)?);
                } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Static) {
                    self.consume()?;
                    if self.consume_if_keyword(Keyword::Const)?.is_some() {
                        var_decls.extend(self.parse_var_decl_list(VarDeclKind::StaticConst)?);
                    } else {
                        var_decls.extend(self.parse_var_decl_list(VarDeclKind::Static)?);
                    }
                } else {
                    let (function, error) = self.parse_function_recovering()?;
                    functions.push(function);
                    if let Some(error) = error {
                        diagnostics.push(error);
                    }
                }
                Ok(())
            })();

            match attempt {
                Ok(()) => top_level_ok = true,
                Err(error) => {
                    if top_level_ok {
                        diagnostics.push(error);
                    }
                    top_level_ok = false;
                    if self.consumed_tokens == declaration_start {
                        self.discard_one_recovery_token();
                    }
                }
            }
        }

        (
            Script::with_directives(functions, var_decls, includes, appends, strict_level),
            diagnostics,
        )
    }

    #[allow(dead_code)]
    fn parse_function(&mut self) -> Result<Function, ParseError> {
        // Parse optional access modifier (private/protected/public/global)
        // Default is public if no modifier specified
        let access = if self.consume_if_keyword(Keyword::Private)?.is_some() {
            AccessLevel::Private
        } else if self.consume_if_keyword(Keyword::Protected)?.is_some() {
            AccessLevel::Protected
        } else if self.consume_if_keyword(Keyword::Public)?.is_some() {
            AccessLevel::Public
        } else if self.consume_if_keyword(Keyword::Global)?.is_some() {
            AccessLevel::Global
        } else {
            AccessLevel::Public // Default access level
        };

        if !self.check_keyword(Keyword::Func)? {
            return self.parse_old_style_function(access);
        }
        self.expect_keyword(Keyword::Func, "expected 'func' declaration")?;
        // Check for optional & indicating reference return type
        let returns_reference = self.consume_if_symbol(Symbol::Ampersand)?.is_some();
        let (name, _) = self.expect_identifier("expected function name")?;
        self.expect_symbol(Symbol::LParen, "expected '(' after function name")?;
        let params = self.parse_parameter_list()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after parameter list")?;
        self.expect_symbol(Symbol::LBrace, "expected '{' to start function body")?;
        let description = self.parse_function_description()?;
        let body = self.parse_block_statements()?;
        self.expect_symbol(Symbol::RBrace, "expected '}' after function body")?;

        Ok(Function {
            name,
            params,
            body,
            access,
            returns_reference,
            description,
            // Stamped with the script's #strict level in Script::from_ast.
            strict_level: None,
            // Linked when a later script or an #include overload collides.
            overloaded: None,
        })
    }

    fn parse_function_recovering(&mut self) -> Result<(Function, Option<ParseError>), ParseError> {
        let access = if self.consume_if_keyword(Keyword::Private)?.is_some() {
            AccessLevel::Private
        } else if self.consume_if_keyword(Keyword::Protected)?.is_some() {
            AccessLevel::Protected
        } else if self.consume_if_keyword(Keyword::Public)?.is_some() {
            AccessLevel::Public
        } else if self.consume_if_keyword(Keyword::Global)?.is_some() {
            AccessLevel::Global
        } else {
            AccessLevel::Public
        };

        if !self.check_keyword(Keyword::Func)? {
            return self.parse_old_style_function_recovering(access);
        }
        self.expect_keyword(Keyword::Func, "expected 'func' declaration")?;
        let returns_reference = self.consume_if_symbol(Symbol::Ampersand)?.is_some();
        let (name, _) = self.expect_identifier("expected function name")?;
        self.expect_symbol(Symbol::LParen, "expected '(' after function name")?;
        let params = self.parse_parameter_list()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after parameter list")?;
        self.expect_symbol(Symbol::LBrace, "expected '{' to start function body")?;
        let body_depth = self.brace_depth;

        let mut description = None;
        let mut body = Vec::new();
        let error = match self.parse_function_description() {
            Ok(parsed) => {
                description = parsed;
                let (parsed_body, error) = self.parse_block_statements_until_error();
                body = parsed_body;
                match error {
                    Some(error) => Some(error),
                    None => self
                        .expect_symbol(Symbol::RBrace, "expected '}' after function body")
                        .err(),
                }
            }
            Err(error) => Some(error),
        };

        if let Some(error) = &error {
            self.recover_function_body(body_depth);
            body.push(Stmt::ParseError {
                message: error.message().to_string(),
                line: error.line(),
                column: error.column(),
            });
        }

        Ok((
            Function {
                name,
                params,
                body,
                access,
                returns_reference,
                description,
                strict_level: None,
                overloaded: None,
            },
            error,
        ))
    }

    fn parse_old_style_function(&mut self, access: AccessLevel) -> Result<Function, ParseError> {
        let (name, name_token) = self.expect_identifier("expected function declaration")?;
        if self.strict_level >= 2 {
            return Err(ParseError::new(
                format!("declaration expected, but found identifier '{name}'"),
                name_token.line,
                name_token.column,
            ));
        }
        self.expect_symbol(Symbol::Colon, "expected ':' after old-style function name")?;
        let description = self.parse_function_description()?;

        let mut body = Vec::new();
        while !self.is_eof()? && !self.is_old_style_function_boundary()? {
            body.push(self.parse_statement()?);
        }

        Ok(Function {
            name,
            params: Vec::new(),
            body,
            access,
            returns_reference: false,
            description,
            strict_level: None,
            overloaded: None,
        })
    }

    fn parse_old_style_function_recovering(
        &mut self,
        access: AccessLevel,
    ) -> Result<(Function, Option<ParseError>), ParseError> {
        let (name, name_token) = self.expect_identifier("expected function declaration")?;
        if self.strict_level >= 2 {
            return Err(ParseError::new(
                format!("declaration expected, but found identifier '{name}'"),
                name_token.line,
                name_token.column,
            ));
        }
        self.expect_symbol(Symbol::Colon, "expected ':' after old-style function name")?;
        let body_depth = self.brace_depth;

        let mut description = None;
        let mut body = Vec::new();
        let mut error = match self.parse_function_description() {
            Ok(parsed) => {
                description = parsed;
                None
            }
            Err(error) => Some(error),
        };

        while error.is_none() {
            match self.is_eof() {
                Ok(true) => break,
                Ok(false) => {}
                Err(parse_error) => {
                    error = Some(parse_error);
                    break;
                }
            }
            match self.is_old_style_function_boundary() {
                Ok(true) => break,
                Ok(false) => {}
                Err(parse_error) => {
                    error = Some(parse_error);
                    break;
                }
            }
            match self.parse_statement() {
                Ok(statement) => body.push(statement),
                Err(parse_error) => error = Some(parse_error),
            }
        }

        if let Some(parse_error) = &error {
            self.recover_old_style_function_body(body_depth);
            body.push(Stmt::ParseError {
                message: parse_error.message().to_string(),
                line: parse_error.line(),
                column: parse_error.column(),
            });
        }

        Ok((
            Function {
                name,
                params: Vec::new(),
                body,
                access,
                returns_reference: false,
                description,
                strict_level: None,
                overloaded: None,
            },
            error,
        ))
    }

    /// Old-format functions end at EOF/directives, a new-format declaration,
    /// an access modifier, or the next `Name:` label
    /// (C4AulParse.cpp:1760-1805, 2167-2188, 2220-2238).
    fn is_old_style_function_boundary(&mut self) -> Result<bool, ParseError> {
        match &self.peek()?.kind {
            TokenKind::Eof | TokenKind::Directive(_) => return Ok(true),
            TokenKind::Keyword(
                Keyword::Private
                | Keyword::Protected
                | Keyword::Public
                | Keyword::Global
                | Keyword::Func,
            ) => return Ok(true),
            TokenKind::Identifier(_) | TokenKind::C4Id(_) | TokenKind::Keyword(_) => {}
            _ => return Ok(false),
        }

        self.begin_speculative();
        let result = (|| {
            self.consume()?;
            self.check_symbol(Symbol::Colon)
        })();
        self.reset_speculative();
        result
    }

    fn parse_parameter_list(&mut self) -> Result<Vec<Parameter>, ParseError> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen)? {
            return Ok(params);
        }
        loop {
            // `...` ends the parameter list: the function takes anything via
            // Par() and declares no further names (C4AulParse.cpp:1642-1648).
            if self.consume_if_symbol(Symbol::Ellipsis)?.is_some() {
                break;
            }

            // Check for optional type annotation
            let type_annotation = self.parse_type_annotation()?;

            // Check for optional reference parameter (&)
            let is_reference = self.consume_if_symbol(Symbol::Ampersand)?.is_some();

            // Try to get parameter name
            // If we have a type annotation but the next token is not an identifier (nor &),
            // then the "type" was actually the parameter name itself
            let next_token = self.peek()?;
            let (name, actual_type) = match &next_token.kind {
                TokenKind::Identifier(param_name) => {
                    let name = param_name.clone();
                    self.consume()?;
                    (name, type_annotation)
                }
                // C4Aul keywords are contextual (plain ATT_IDTF words in the
                // C++ tokenizer), so any keyword is a legal parameter name —
                // `func SetPrivateTeleporter(bool private)` (Hazard
                // Teleporter.c4d/Script.c:238).
                TokenKind::Keyword(keyword) => {
                    let name = keyword.lexeme().to_string();
                    self.consume()?;
                    (name, type_annotation)
                }
                _ if type_annotation.is_some() => {
                    // The type annotation token was actually the parameter name
                    // (e.g., "effect" in "func Foo(effect, target)")
                    let param_name = type_annotation.as_ref().unwrap().to_string();
                    (param_name, None)
                }
                _ => {
                    let line = next_token.line;
                    let column = next_token.column;
                    return Err(ParseError::new("expected parameter name", line, column));
                }
            };

            // C4Aul stores parameter names in C4ValueMapNames. AddName
            // returns the existing slot for a duplicate instead of adding a
            // new positional name (C4AulParse.cpp:1677-1682;
            // C4ValueMap.cpp:406-411). A few shipped legacy callbacks rely
            // on the following unique name shifting into that reused slot.
            if !params.iter().any(|parameter| parameter.name == name) {
                if is_reference || actual_type.is_some() {
                    params.push(Parameter::with_reference(name, actual_type, is_reference));
                } else {
                    params.push(Parameter::new(name));
                }
            }

            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }
        Ok(params)
    }

    fn parse_type_annotation(&mut self) -> Result<Option<TypeAnnotation>, ParseError> {
        let token = self.peek()?;
        let base_type = match &token.kind {
            // Type keywords are contextual - check identifier names
            TokenKind::Identifier(name) => match name.as_str() {
                "int" => {
                    self.consume()?;
                    TypeAnnotation::Int
                }
                "bool" => {
                    self.consume()?;
                    TypeAnnotation::Bool
                }
                "string" => {
                    self.consume()?;
                    TypeAnnotation::String
                }
                "object" => {
                    self.consume()?;
                    TypeAnnotation::Object
                }
                "id" => {
                    self.consume()?;
                    TypeAnnotation::Id
                }
                "array" => {
                    self.consume()?;
                    TypeAnnotation::Array
                }
                "proplist" => {
                    self.consume()?;
                    TypeAnnotation::Proplist
                }
                "effect" => {
                    self.consume()?;
                    TypeAnnotation::Effect
                }
                "any" => {
                    self.consume()?;
                    TypeAnnotation::Any
                }
                _ => return Ok(None),
            },
            TokenKind::Keyword(Keyword::Nil) => {
                self.consume()?;
                TypeAnnotation::Nil
            }
            _ => return Ok(None), // No type annotation
        };

        // Check for union types (e.g., object|nil)
        if self.check_symbol(Symbol::Pipe)? {
            let mut types = vec![base_type];
            while self.consume_if_symbol(Symbol::Pipe)?.is_some() {
                let next_token = self.peek()?;
                let next_type = match &next_token.kind {
                    // Type keywords are contextual - check identifier names
                    TokenKind::Identifier(name) => match name.as_str() {
                        "int" => {
                            self.consume()?;
                            TypeAnnotation::Int
                        }
                        "bool" => {
                            self.consume()?;
                            TypeAnnotation::Bool
                        }
                        "string" => {
                            self.consume()?;
                            TypeAnnotation::String
                        }
                        "object" => {
                            self.consume()?;
                            TypeAnnotation::Object
                        }
                        "id" => {
                            self.consume()?;
                            TypeAnnotation::Id
                        }
                        "array" => {
                            self.consume()?;
                            TypeAnnotation::Array
                        }
                        "proplist" => {
                            self.consume()?;
                            TypeAnnotation::Proplist
                        }
                        "effect" => {
                            self.consume()?;
                            TypeAnnotation::Effect
                        }
                        "any" => {
                            self.consume()?;
                            TypeAnnotation::Any
                        }
                        _ => {
                            return Err(ParseError::new(
                                "expected type name after '|' in union type".to_string(),
                                next_token.line,
                                next_token.column,
                            ))
                        }
                    },
                    TokenKind::Keyword(Keyword::Nil) => {
                        self.consume()?;
                        TypeAnnotation::Nil
                    }
                    _ => {
                        return Err(ParseError::new(
                            "expected type name after '|' in union type".to_string(),
                            next_token.line,
                            next_token.column,
                        ))
                    }
                };
                types.push(next_type);
            }
            Ok(Some(TypeAnnotation::Union(types)))
        } else {
            Ok(Some(base_type))
        }
    }

    fn parse_block_statements(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut statements = Vec::new();
        while !self.check_symbol(Symbol::RBrace)? && !self.is_eof()? {
            statements.push(self.parse_statement()?);
        }
        Ok(statements)
    }

    /// Parse as much of a function body as possible. C++ retains bytecode
    /// emitted before a parser failure, then appends AB_ERR; preserving the
    /// statement prefix gives the tree-walking VM the same observable order.
    fn parse_block_statements_until_error(&mut self) -> (Vec<Stmt>, Option<ParseError>) {
        let mut statements = Vec::new();
        loop {
            match self.check_symbol(Symbol::RBrace) {
                Ok(true) => return (statements, None),
                Ok(false) => {}
                Err(error) => return (statements, Some(error)),
            }
            match self.is_eof() {
                Ok(true) => return (statements, None),
                Ok(false) => {}
                Err(error) => return (statements, Some(error)),
            }
            match self.parse_statement() {
                Ok(statement) => statements.push(statement),
                Err(error) => return (statements, Some(error)),
            }
        }
    }

    fn recover_function_body(&mut self, body_depth: usize) {
        while self.brace_depth >= body_depth {
            match self.consume() {
                Ok(token) => {
                    if matches!(token.kind, TokenKind::Eof) {
                        break;
                    }
                }
                // The lexer advanced past the bad bytes before returning
                // its error. Clear any cached token and keep scanning.
                Err(_) => self.peeked = None,
            }
        }
    }

    fn recover_old_style_function_body(&mut self, body_depth: usize) {
        loop {
            if self.brace_depth <= body_depth {
                match self.is_eof() {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) => {
                        self.peeked = None;
                        continue;
                    }
                }
                match self.is_old_style_function_boundary() {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) => {
                        self.peeked = None;
                        continue;
                    }
                }
            }

            match self.consume() {
                Ok(token) => {
                    if matches!(token.kind, TokenKind::Eof) {
                        break;
                    }
                }
                Err(_) => self.peeked = None,
            }
        }
    }

    fn parse_function_description(&mut self) -> Result<Option<String>, ParseError> {
        if !self.check_symbol(Symbol::LBracket)? {
            return Ok(None);
        }

        let opening = self.consume()?;
        self.lexer
            .skip_function_description(opening.line, opening.column)
            .map(Some)
    }

    fn parse_stmt_or_block_vec(&mut self) -> Result<Vec<Stmt>, ParseError> {
        // Parse a single statement. If it was a braced block, unwrap it to Vec<Stmt>,
        // otherwise wrap the single statement into a one-element Vec.
        let stmt = self.parse_statement()?;
        match stmt {
            Stmt::Block(body) => Ok(body),
            other => Ok(vec![other]),
        }
    }

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        if self.consume_if_keyword(Keyword::Var)?.is_some() {
            return self.parse_var_decl();
        }
        if self.consume_if_keyword(Keyword::Return)?.is_some() {
            return self.parse_return();
        }
        if self.consume_if_keyword(Keyword::Break)?.is_some() {
            self.expect_symbol(Symbol::Semicolon, "expected ';' after break")?;
            return Ok(Stmt::Break);
        }
        if self.consume_if_keyword(Keyword::Continue)?.is_some() {
            self.expect_symbol(Symbol::Semicolon, "expected ';' after continue")?;
            return Ok(Stmt::Continue);
        }
        if self.consume_if_keyword(Keyword::If)?.is_some() {
            return self.parse_if();
        }
        if self.consume_if_keyword(Keyword::While)?.is_some() {
            return self.parse_while();
        }
        if self.consume_if_keyword(Keyword::For)?.is_some() {
            return self.parse_for();
        }
        if self.consume_if_symbol(Symbol::LBrace)?.is_some() {
            let body = self.parse_block_statements()?;
            self.expect_symbol(Symbol::RBrace, "expected '}' to close block")?;
            return Ok(Stmt::Block(body));
        }

        // Handle empty statement (;)
        // Must check before parse_assignment_or_expr() because ; is not a valid expression start
        if self.consume_if_symbol(Symbol::Semicolon)?.is_some() {
            return Ok(Stmt::Block(Vec::new())); // Empty block represents empty statement
        }

        self.parse_assignment_or_expr()
    }

    fn parse_var_decl(&mut self) -> Result<Stmt, ParseError> {
        let mut decls = Vec::new();

        loop {
            // Parse one variable
            let (name, _) = self.expect_identifier("expected variable name")?;
            let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                // Use parse_assignment() instead of parse_expression() to avoid comma operator
                // In variable declarations, commas separate variables, not comma expressions
                Some(self.parse_assignment()?)
            } else {
                None
            };
            decls.push(Stmt::VarDecl { name, init });

            // Check what's next: comma means more variables, otherwise expect semicolon
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }

        self.expect_symbol(Symbol::Semicolon, "expected ';' after variable declaration")?;

        // Return single declaration if only one, otherwise return sequence
        // Sequence executes statements without creating a new scope (unlike Block)
        if decls.len() == 1 {
            Ok(decls.into_iter().next().unwrap())
        } else {
            Ok(Stmt::Sequence(decls))
        }
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        // Handle: return;
        if self.consume_if_symbol(Symbol::Semicolon)?.is_some() {
            return Ok(Stmt::Return(None));
        }

        // Before #strict 2, C4Aul has a compatibility-only multi-parameter
        // `return (first, unused, ...)` form. Tokenization discards whitespace,
        // so adjacency to `return` is irrelevant. Probe for a top-level comma
        // first: a single `(expr)` must stay on the normal expression path so
        // suffixes such as `return (1 + 1) * 3` keep parsing.
        if self.strict_level < 2 && self.parenthesized_return_has_top_level_comma()? {
            self.consume()?; // consume '('

            // Handle: return(); (empty parentheses)
            if self.check_symbol(Symbol::RParen)? {
                self.consume()?; // consume ')'
                self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;
                return Ok(Stmt::Return(None));
            }

            // Parse assignments here rather than parse_expression(): at this
            // level commas delimit the legacy parameters instead of becoming
            // the normal comma operator.
            let mut exprs = vec![self.parse_assignment()?];
            while self.consume_if_symbol(Symbol::Comma)?.is_some() {
                exprs.push(self.parse_assignment()?);
            }
            self.expect_symbol(Symbol::RParen, "expected ')' after expression")?;
            self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;

            let value = if exprs.len() == 1 {
                exprs.pop().unwrap()
            } else {
                // Array elements evaluate left-to-right. Selecting element
                // zero preserves every unused parameter's side effects while
                // yielding the first value, matching the old C4Aul bytecode.
                Expr::Index(
                    Box::new(Expr::Array(exprs)),
                    Box::new(Expr::Literal(Literal::Int(0))),
                )
            };
            return Ok(Stmt::Return(Some(value)));
        }

        // Handle: return (); (with space between return and ())
        if self.check_symbol(Symbol::LParen)? {
            let lparen_token = self.consume()?; // consume '('

            if self.check_symbol(Symbol::RParen)? {
                // Empty parens - consume ')' and ';', return None
                self.consume()?; // consume ')'
                self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;
                return Ok(Stmt::Return(None));
            }

            // Not empty parens - put back both the peeked token and the '('
            // We need to restore both because we consumed '(' and peeked at the next token
            if let Some(peeked_token) = self.peeked.take() {
                self.lookahead_buffer.insert(0, peeked_token);
            }
            self.rewind_consumed_token(&lparen_token);
            self.lookahead_buffer.insert(0, lparen_token);
            self.peeked = None;
        }

        // Handle: return expr; (normal expression parsing, includes "return (expr) op expr;")
        let expr = self.parse_expression()?;
        self.expect_symbol(Symbol::Semicolon, "expected ';' after return value")?;
        Ok(Stmt::Return(Some(expr)))
    }

    fn parenthesized_return_has_top_level_comma(&mut self) -> Result<bool, ParseError> {
        if !self.check_symbol(Symbol::LParen)? {
            return Ok(false);
        }

        self.begin_speculative();
        let result = (|| {
            self.consume()?; // outer '('
            let mut paren_depth = 1usize;
            let mut bracket_depth = 0usize;
            let mut brace_depth = 0usize;

            loop {
                let token = self.consume()?;
                match token.kind {
                    TokenKind::Symbol(Symbol::LParen) => paren_depth += 1,
                    TokenKind::Symbol(Symbol::RParen) if paren_depth == 1 => return Ok(false),
                    TokenKind::Symbol(Symbol::RParen) => paren_depth -= 1,
                    TokenKind::Symbol(Symbol::LBracket) => bracket_depth += 1,
                    TokenKind::Symbol(Symbol::RBracket) => {
                        bracket_depth = bracket_depth.saturating_sub(1)
                    }
                    TokenKind::Symbol(Symbol::LBrace) => brace_depth += 1,
                    TokenKind::Symbol(Symbol::RBrace) => {
                        brace_depth = brace_depth.saturating_sub(1)
                    }
                    TokenKind::Symbol(Symbol::Comma)
                        if paren_depth == 1 && bracket_depth == 0 && brace_depth == 0 =>
                    {
                        return Ok(true)
                    }
                    TokenKind::Eof => return Ok(false),
                    _ => {}
                }
            }
        })();
        self.reset_speculative();
        result
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.expect_symbol(Symbol::LParen, "expected '(' after 'if'")?;
        let condition = self.parse_expression()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after condition")?;

        // Parse either a single statement or a braced block for the 'then' part.
        let then_branch = self.parse_stmt_or_block_vec()?;

        // Else binds to nearest if: if the 'then' was itself an if with an else,
        // that else would already be consumed by parse_statement().
        let else_branch = if self.consume_if_keyword(Keyword::Else)?.is_some() {
            Some(self.parse_stmt_or_block_vec()?)
        } else {
            None
        };

        Ok(Stmt::If {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect_symbol(Symbol::LParen, "expected '(' after 'while'")?;
        let condition = self.parse_expression()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after condition")?;

        // Parse either a single statement or a braced block for the loop body.
        let body = self.parse_stmt_or_block_vec()?;

        Ok(Stmt::While { condition, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        use crate::ast::ForInit;

        self.expect_symbol(Symbol::LParen, "expected '(' after 'for'")?;

        // Distinguish for-in from C-style for
        // for-in: for(var obj in expr) or for(obj in expr)
        // C-style: for(init; cond; incr)

        // Check if this starts with 'var'
        if self.check_keyword(Keyword::Var)? {
            self.consume()?; // consume 'var'

            // Parse the identifier
            let (variable, _) = self.expect_identifier("expected variable name")?;

            // Check next token to distinguish for-in from C-style
            if self.consume_if_keyword(Keyword::In)?.is_some() {
                // For-in loop: for(var variable in iterable)
                let iterable = self.parse_expression()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after for-in header")?;
                let body = self.parse_stmt_or_block_vec()?;

                return Ok(Stmt::ForIn {
                    variable,
                    declare_var: true,
                    iterable,
                    body,
                });
            } else {
                // C-style for loop: for(var i = 0, j = 1; ...)
                // We've already consumed 'var' and the first identifier
                // Continue parsing as var decl list
                let mut decls = Vec::new();

                // Check for initializer on first variable
                let first_init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                    // Use parse_assignment() instead of parse_expression() to avoid comma operator
                    // In variable declarations, commas separate variables, not comma expressions
                    Some(self.parse_assignment()?)
                } else {
                    None
                };
                decls.push((variable, first_init));

                // Parse additional comma-separated variables
                while self.consume_if_symbol(Symbol::Comma)?.is_some() {
                    let (name, _) = self.expect_identifier("expected variable name")?;
                    let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                        // Use parse_assignment() instead of parse_expression() to avoid comma operator
                        // In variable declarations, commas separate variables, not comma expressions
                        Some(self.parse_assignment()?)
                    } else {
                        None
                    };
                    decls.push((name, init));
                }

                let init = Some(ForInit::VarDecls(decls));

                self.expect_symbol(Symbol::Semicolon, "expected ';' after for-init")?;

                // Parse condition clause (optional)
                let condition = if self.check_symbol(Symbol::Semicolon)? {
                    None
                } else {
                    Some(self.parse_expression()?)
                };

                self.expect_symbol(Symbol::Semicolon, "expected ';' after for-condition")?;

                // Parse increment clause (optional)
                let increment = if self.check_symbol(Symbol::RParen)? {
                    None
                } else {
                    Some(self.parse_expression()?)
                };

                self.expect_symbol(Symbol::RParen, "expected ')' after for-clauses")?;

                // Parse body
                let body = self.parse_stmt_or_block_vec()?;

                return Ok(Stmt::For {
                    init,
                    condition,
                    increment,
                    body,
                });
            }
        }

        // No 'var' - could be for-in with pre-declared variable or C-style for
        // Need 2-token lookahead: identifier + 'in' means for-in, otherwise C-style
        if self.check_identifier()? {
            // Get first token (identifier) by consuming
            let token1 = self.consume()?;

            // Get second token by consuming
            let token2 = self.consume()?;

            // Check if it's for-in pattern
            let is_for_in = matches!(token2.kind, TokenKind::Keyword(Keyword::In));

            if is_for_in {
                // For-in: for(variable in iterable)
                let variable = if let TokenKind::Identifier(name) = token1.kind {
                    name
                } else {
                    unreachable!()
                };

                // Both tokens are already consumed, just continue parsing
                let iterable = self.parse_expression()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after for-in header")?;
                let body = self.parse_stmt_or_block_vec()?;

                return Ok(Stmt::ForIn {
                    variable,
                    declare_var: false,
                    iterable,
                    body,
                });
            } else {
                // C-style for: restore both tokens
                // Push in reverse order so token1 comes out first
                self.rewind_consumed_token(&token2);
                self.rewind_consumed_token(&token1);
                self.lookahead_buffer.insert(0, token2);
                self.lookahead_buffer.insert(0, token1);
                // Clear peeked to ensure restored tokens are seen first
                self.peeked = None;
            }
        }

        // C-style for loop: for(init; cond; incr) or for(; cond; incr)
        let init = if self.check_symbol(Symbol::Semicolon)? {
            None
        } else {
            // Parse expression as init
            Some(ForInit::Expr(self.parse_expression()?))
        };

        self.expect_symbol(Symbol::Semicolon, "expected ';' after for-init")?;

        // Parse condition clause (optional)
        let condition = if self.check_symbol(Symbol::Semicolon)? {
            None
        } else {
            Some(self.parse_expression()?)
        };

        self.expect_symbol(Symbol::Semicolon, "expected ';' after for-condition")?;

        // Parse increment clause (optional)
        let increment = if self.check_symbol(Symbol::RParen)? {
            None
        } else {
            Some(self.parse_expression()?)
        };

        self.expect_symbol(Symbol::RParen, "expected ')' after for-clauses")?;

        // Parse body
        let body = self.parse_stmt_or_block_vec()?;

        Ok(Stmt::For {
            init,
            condition,
            increment,
            body,
        })
    }

    fn parse_assignment_or_expr(&mut self) -> Result<Stmt, ParseError> {
        let expr = self.parse_expression()?;
        // At statement level, we always expect a trailing semicolon
        self.expect_symbol(Symbol::Semicolon, "expected ';' after expression")?;

        match expr {
            Expr::Assignment(target, value) => Ok(Stmt::Assignment {
                target,
                value: *value,
            }),
            _ => Ok(Stmt::Expr(expr)),
        }
    }

    fn expression_to_assignment_target(
        expr: Expr,
        eq_token: &Token,
    ) -> Result<AssignmentTarget, ParseError> {
        match expr {
            Expr::Variable(name) => Ok(AssignmentTarget::Variable(name)),
            Expr::Property(base, name) => {
                let base_target = Self::expression_to_assignment_target(*base, eq_token)?;
                Ok(AssignmentTarget::Property(Box::new(base_target), name))
            }
            // Array/proplist indexing is assignable: arr[i] = value
            Expr::Index(base, index) => {
                let base_target = Self::expression_to_assignment_target(*base, eq_token)?;
                Ok(AssignmentTarget::Index(Box::new(base_target), index))
            }
            // Special case: Local(expr), Var(expr), and EffectVar(args...) are assignable lvalues
            // Local() and Var() without arguments default to slot 0
            Expr::Call {
                callee,
                args,
                is_optional,
                ..
            } => {
                if let Expr::Variable(ref name) = *callee {
                    if !is_optional {
                        if name == "Local" && (args.is_empty() || args.len() == 1) {
                            let index = if args.is_empty() {
                                Box::new(Expr::Literal(Literal::Int(0)))
                            } else {
                                Box::new(args.into_iter().next().unwrap())
                            };
                            return Ok(AssignmentTarget::LocalSlot(index));
                        } else if name == "Var" && (args.is_empty() || args.len() == 1) {
                            let index = if args.is_empty() {
                                Box::new(Expr::Literal(Literal::Int(0)))
                            } else {
                                Box::new(args.into_iter().next().unwrap())
                            };
                            return Ok(AssignmentTarget::VarSlot(index));
                        } else if name == "EffectVar" {
                            // EffectVar can take any number of arguments
                            return Ok(AssignmentTarget::EffectSlot(args));
                        } else if (name == "Local" || name == "LocalN" || name == "Var")
                            && args.len() == 2
                        {
                            // Handle two-argument form: LocalN("key", obj), Local(index, obj), Var(index, obj)
                            // Convert to MethodSlot: obj->LocalN("key")
                            let mut args_iter = args.into_iter();
                            let first_arg = args_iter.next().unwrap();
                            let object = args_iter.next().unwrap();
                            return Ok(AssignmentTarget::MethodSlot {
                                object: Box::new(object),
                                method: name.clone(),
                                args: vec![first_arg],
                            });
                        }
                        // NEW: Allow any function call as a potential lvalue
                        // This supports user-defined reference-returning functions (func &)
                        return Ok(AssignmentTarget::FunctionCall {
                            name: name.clone(),
                            args,
                        });
                    }
                }
                // AB_CALL leaves a reference return intact for AB_Set. Any
                // non-failsafe arrow call is therefore a syntactic lvalue
                // candidate; the runtime validates that its callee is `func &`.
                else if let Expr::Property(ref object, ref method) = *callee {
                    if !is_optional {
                        return Ok(AssignmentTarget::MethodSlot {
                            object: object.clone(),
                            method: method.clone(),
                            args,
                        });
                    }
                }
                Err(ParseError::new(
                    "invalid assignment target",
                    eq_token.line,
                    eq_token.column,
                ))
            }
            _ => Err(ParseError::new(
                "invalid assignment target",
                eq_token.line,
                eq_token.column,
            )),
        }
    }

    fn validate_lvalue(&self, expr: &Expr, token: &Token) -> Result<(), ParseError> {
        match expr {
            Expr::Variable(_) | Expr::Property(_, _) | Expr::Index(_, _) => Ok(()),
            // Prefix increment/decrement return lvalues (like in C++)
            // This allows patterns like ++++i or --(--i)
            Expr::PreIncrement(_) | Expr::PreDecrement(_) => Ok(()),
            // Special cases: Local/LocalN/Var/EffectVar are valid for increment/decrement
            // Also allow any function call (for reference-returning functions)
            Expr::Call { callee, is_optional, .. } => {
                if let Expr::Variable(_) = **callee {
                    if !is_optional {
                        // Allow any non-optional function call to be used with increment/decrement
                        // This supports both built-in functions (Local, Var, etc.) and
                        // user-defined reference-returning functions (func &)
                        return Ok(());
                    }
                }
                Err(ParseError::new(
                    "increment/decrement requires an lvalue (variable, property, index, Local(n), or Var(n))",
                    token.line,
                    token.column,
                ))
            }
            _ => Err(ParseError::new(
                "increment/decrement requires an lvalue (variable, property, index, Local(n), or Var(n))",
                token.line,
                token.column,
            )),
        }
    }

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_comma()
    }

    fn parse_comma(&mut self) -> Result<Expr, ParseError> {
        let mut exprs = vec![self.parse_assignment()?];

        while self.consume_if_symbol(Symbol::Comma)?.is_some() {
            exprs.push(self.parse_assignment()?);
        }

        if exprs.len() == 1 {
            Ok(exprs.into_iter().next().unwrap())
        } else {
            Ok(Expr::Comma(exprs))
        }
    }

    fn parse_assignment(&mut self) -> Result<Expr, ParseError> {
        // Parse the next higher-precedence level first
        let left = self.parse_nil_coalescing()?;

        // Check for assignment operators (=, +=, -=, etc.)
        let maybe_op = self.peek()?;
        let (is_assign, op_symbol) = match &maybe_op.kind {
            TokenKind::Symbol(Symbol::Equal) => (true, Some(Symbol::Equal)),
            TokenKind::Symbol(Symbol::PlusEqual) => (true, Some(Symbol::PlusEqual)),
            TokenKind::Symbol(Symbol::MinusEqual) => (true, Some(Symbol::MinusEqual)),
            TokenKind::Symbol(Symbol::StarEqual) => (true, Some(Symbol::StarEqual)),
            TokenKind::Symbol(Symbol::SlashEqual) => (true, Some(Symbol::SlashEqual)),
            TokenKind::Symbol(Symbol::PercentEqual) => (true, Some(Symbol::PercentEqual)),
            TokenKind::Symbol(Symbol::AndEqual) => (true, Some(Symbol::AndEqual)),
            TokenKind::Symbol(Symbol::OrEqual) => (true, Some(Symbol::OrEqual)),
            TokenKind::Symbol(Symbol::XorEqual) => (true, Some(Symbol::XorEqual)),
            TokenKind::Symbol(Symbol::LeftShiftEqual) => (true, Some(Symbol::LeftShiftEqual)),
            TokenKind::Symbol(Symbol::RightShiftEqual) => (true, Some(Symbol::RightShiftEqual)),
            TokenKind::Symbol(Symbol::ConcatEqual) => (true, Some(Symbol::ConcatEqual)),
            TokenKind::Symbol(Symbol::QuestionQuestionEqual) => {
                (true, Some(Symbol::QuestionQuestionEqual))
            }
            _ => (false, None),
        };

        if is_assign {
            let op_token = self.consume()?;
            // Validate the left side is a legal assignment target
            let target = Self::expression_to_assignment_target(left.clone(), &op_token)?;

            // Right-associative: a = b = c parses as a = (b = c)
            let value = self.parse_assignment()?;

            // Desugar compound assignments: a += b becomes a = a + b
            let final_value = match op_symbol.unwrap() {
                Symbol::Equal => value,
                Symbol::PlusEqual => Expr::Binary(Box::new(left), BinaryOp::Add, Box::new(value)),
                Symbol::MinusEqual => Expr::Binary(Box::new(left), BinaryOp::Sub, Box::new(value)),
                Symbol::StarEqual => Expr::Binary(Box::new(left), BinaryOp::Mul, Box::new(value)),
                Symbol::SlashEqual => Expr::Binary(Box::new(left), BinaryOp::Div, Box::new(value)),
                Symbol::PercentEqual => {
                    Expr::Binary(Box::new(left), BinaryOp::Mod, Box::new(value))
                }
                Symbol::ConcatEqual => {
                    Expr::Binary(Box::new(left), BinaryOp::Concat, Box::new(value))
                }
                // `a ??= b` ≙ `a = a ?? b` (AB_NilCoalescingIt,
                // C4AulParse.cpp:477): `??`'s short-circuit keeps the rhs
                // unevaluated when `a` is non-nil.
                Symbol::QuestionQuestionEqual => {
                    Expr::Binary(Box::new(left), BinaryOp::NilCoalescing, Box::new(value))
                }
                // Bitwise compound assignments
                Symbol::AndEqual => Expr::Binary(Box::new(left), BinaryOp::BitAnd, Box::new(value)),
                Symbol::OrEqual => Expr::Binary(Box::new(left), BinaryOp::BitOr, Box::new(value)),
                Symbol::XorEqual => Expr::Binary(Box::new(left), BinaryOp::BitXor, Box::new(value)),
                Symbol::LeftShiftEqual => {
                    Expr::Binary(Box::new(left), BinaryOp::LeftShift, Box::new(value))
                }
                Symbol::RightShiftEqual => {
                    Expr::Binary(Box::new(left), BinaryOp::RightShift, Box::new(value))
                }
                _ => {
                    return Err(ParseError::new(
                        format!("unknown assignment operator {:?}", op_symbol.unwrap()),
                        op_token.line,
                        op_token.column,
                    ))
                }
            };

            Ok(Expr::Assignment(target, Box::new(final_value)))
        } else {
            Ok(left)
        }
    }

    /// `??` sits between `||` (priority 4) and the assignments (priority 2)
    /// in C4ScriptOpMap (C4AulParse.cpp:463-464).
    fn parse_nil_coalescing(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_or()?;
        while self.consume_if_symbol(Symbol::QuestionQuestion)?.is_some() {
            let right = self.parse_or()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::NilCoalescing, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_and()?;
        while self.consume_if_symbol(Symbol::OrOr)?.is_some()
            || self.consume_if_identifier("or")?.is_some()
        {
            let right = self.parse_and()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::Or, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_or()?;
        while self.consume_if_symbol(Symbol::AndAnd)?.is_some()
            || self.consume_if_identifier("and")?.is_some()
        {
            let right = self.parse_bit_or()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::And, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_bit_or(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_xor()?;
        while self.consume_if_symbol(Symbol::Pipe)?.is_some() {
            let right = self.parse_bit_xor()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::BitOr, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_bit_xor(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_and()?;
        while self.consume_if_symbol(Symbol::Caret)?.is_some() {
            let right = self.parse_bit_and()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::BitXor, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_bit_and(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_equality()?;
        while self.consume_if_symbol(Symbol::Ampersand)?.is_some() {
            let right = self.parse_equality()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::BitAnd, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_concat()?;
        loop {
            if self.consume_if_symbol(Symbol::EqualEqual)?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Equal, Box::new(right));
            } else if self.consume_if_symbol(Symbol::BangEqual)?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::NotEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringEqual)?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringEqual, Box::new(right));
            } else if self.consume_if_identifier("eq")?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(
                    Box::new(expr),
                    BinaryOp::KeywordStringEqual,
                    Box::new(right),
                );
            } else if self.consume_if_symbol(Symbol::StringNotEqual)?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringNotEqual, Box::new(right));
            } else if self.consume_if_identifier("ne")?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(
                    Box::new(expr),
                    BinaryOp::KeywordStringNotEqual,
                    Box::new(right),
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// `..` string/array/map concatenation. C4Script priority 10: looser than
    /// comparison (11), tighter than equality (9), so it sits between
    /// parse_equality and parse_comparison.
    fn parse_concat(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_comparison()?;
        while self.consume_if_symbol(Symbol::Concat)?.is_some() {
            let right = self.parse_comparison()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::Concat, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_shift()?;
        loop {
            if self.consume_if_symbol(Symbol::Less)?.is_some()
                || self.consume_if_identifier("lt")?.is_some()
            {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Less, Box::new(right));
            } else if self.consume_if_symbol(Symbol::LessEqual)?.is_some()
                || self.consume_if_identifier("le")?.is_some()
            {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::LessEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Greater)?.is_some()
                || self.consume_if_identifier("gt")?.is_some()
            {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Greater, Box::new(right));
            } else if self.consume_if_symbol(Symbol::GreaterEqual)?.is_some()
                || self.consume_if_identifier("ge")?.is_some()
            {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::GreaterEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringLess)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringLess, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringLessEqual)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringLessEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringGreater)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringGreater, Box::new(right));
            } else if self
                .consume_if_symbol(Symbol::StringGreaterEqual)?
                .is_some()
            {
                let right = self.parse_shift()?;
                expr = Expr::Binary(
                    Box::new(expr),
                    BinaryOp::StringGreaterEqual,
                    Box::new(right),
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_shift(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_term()?;
        loop {
            if self.consume_if_symbol(Symbol::LeftShift)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::LeftShift, Box::new(right));
            } else if self.consume_if_symbol(Symbol::RightShift)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::RightShift, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_factor()?;
        loop {
            if self.consume_if_symbol(Symbol::Plus)?.is_some() {
                let right = self.parse_factor()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Add, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Minus)?.is_some() {
                let right = self.parse_factor()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Sub, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_exponentiation()?;
        loop {
            if self.consume_if_symbol(Symbol::Star)?.is_some() {
                let right = self.parse_exponentiation()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Mul, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Slash)?.is_some() {
                let right = self.parse_exponentiation()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Div, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Percent)?.is_some() {
                let right = self.parse_exponentiation()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Mod, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_exponentiation(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_unary()?;
        // Right-associative: 2**3**2 is parsed as 2**(3**2), not (2**3)**2
        if self.consume_if_symbol(Symbol::StarStar)?.is_some() {
            let right = self.parse_exponentiation()?; // Recursive call for right-associativity
            expr = Expr::Binary(Box::new(expr), BinaryOp::Pow, Box::new(right));
        }
        Ok(expr)
    }

    // Speculative parsing helpers
    fn begin_speculative(&mut self) {
        // Safety check: ensure we're not already in speculative mode
        assert!(
            self.speculative_tokens.is_none(),
            "Nested speculative parsing not supported"
        );
        self.speculative_tokens = Some(Vec::new());
    }

    fn commit_speculative(&mut self) {
        // Clear speculative tokens without restoring
        self.speculative_tokens = None;
    }

    fn reset_speculative(&mut self) {
        if let Some(tokens) = self.speculative_tokens.take() {
            // A peeked-but-unconsumed token is NOT in the speculative
            // record (consume() records at consume time) — it is the
            // stream position right AFTER the recorded tokens, so it goes
            // back into the buffer first (ending up behind the replay).
            if let Some(peeked) = self.peeked.take() {
                self.lookahead_buffer.insert(0, peeked);
            }
            // Restore tokens in reverse order so they come out in the correct order
            for token in tokens.into_iter().rev() {
                self.rewind_consumed_token(&token);
                self.lookahead_buffer.insert(0, token);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        // Check for ! or "not" keyword
        if self.consume_if_symbol(Symbol::Bang)?.is_some()
            || self.consume_if_identifier("not")?.is_some()
        {
            // Only use speculative parsing if we're not already in speculative mode
            // This avoids nested speculative parsing (e.g., !!x or nested expressions)
            let already_speculative = self.speculative_tokens.is_some();

            if !already_speculative {
                // Speculative parse: try parsing assignment expression as operand
                // This allows patterns like: !x = y  →  !(x = y)
                // While preserving precedence for: !a + b  →  (!a) + b

                // NOTE: ! token already consumed, speculative mode tracks only the operand tokens
                self.begin_speculative();

                // Try parsing an assignment expression
                let result = self.parse_assignment();

                // Ensure we always clean up speculative mode
                let final_result = match result {
                    Ok(expr @ Expr::Assignment(..)) => {
                        // The DYNB pattern: !x = y parses as !(x = y).
                        self.commit_speculative();
                        Ok(Expr::Unary(UnaryOp::Not, Box::new(expr)))
                    }
                    Ok(_) => {
                        // C4Aul precedence: `!` binds its unary operand only
                        // (`!A && B` is `(!A) && B`). Committing the full
                        // assignment-level parse here swallowed binary
                        // chains into the negation — the Cowboy Riding
                        // guard fired without evaluating its second
                        // operand (the rider 1425 SetAction recursion).
                        self.reset_speculative();
                        match self.parse_unary() {
                            Ok(operand) => Ok(Expr::Unary(UnaryOp::Not, Box::new(operand))),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => {
                        // Parse failed, reset and try normal precedence (skip ! handling since already consumed)
                        self.reset_speculative();
                        match self.parse_unary() {
                            Ok(operand) => Ok(Expr::Unary(UnaryOp::Not, Box::new(operand))),
                            Err(_) => Err(e), // Return original error
                        }
                    }
                };

                return final_result;
            } else {
                // Already in speculative mode, use normal precedence
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary(UnaryOp::Not, Box::new(expr)));
            }
        }
        if self.consume_if_symbol(Symbol::Minus)?.is_some() {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnaryOp::Negate, Box::new(expr)));
        }
        // Unary plus is a no-op (identity operation)
        if self.consume_if_symbol(Symbol::Plus)?.is_some() {
            return self.parse_unary();
        }
        // Bitwise NOT
        if self.consume_if_symbol(Symbol::Tilde)?.is_some() {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnaryOp::BitwiseNot, Box::new(expr)));
        }
        // Prefix increment/decrement
        if let Some(token) = self.consume_if_symbol(Symbol::PlusPlus)? {
            let expr = self.parse_unary()?;
            self.validate_lvalue(&expr, &token)?;
            return Ok(Expr::PreIncrement(Box::new(expr)));
        }
        if let Some(token) = self.consume_if_symbol(Symbol::MinusMinus)? {
            let expr = self.parse_unary()?;
            self.validate_lvalue(&expr, &token)?;
            return Ok(Expr::PreDecrement(Box::new(expr)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.consume_if_symbol(Symbol::LParen)?.is_some() {
                let (args, forward_rest) = self.parse_argument_list()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    is_optional: false,
                    forward_rest,
                };
            } else if self.consume_if_symbol(Symbol::LBracket)?.is_some() {
                let index = self.parse_expression()?;
                self.expect_symbol(Symbol::RBracket, "expected ']' after index expression")?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.consume_if_symbol(Symbol::Dot)?.is_some() {
                let (name, _) = self.expect_identifier("expected property name after '.'")?;
                expr = Expr::Property(Box::new(expr), name);
            } else if self.consume_if_symbol(Symbol::Arrow)?.is_some() {
                // Check for optional method call: ->~MethodName()
                let is_optional = self.consume_if_symbol(Symbol::Tilde)?.is_some();
                let (mut name, token) =
                    self.expect_identifier_or_c4id("expected property/method name after '->'")?;

                // Check for scope resolution: ->DefID::Method
                if self.consume_if_symbol(Symbol::ColonColon)?.is_some() {
                    let (method_name, _) =
                        self.expect_identifier_or_c4id("expected method name after '::'")?;
                    // Combine as "DefID::Method"
                    name = format!("{}::{}", name, method_name);
                }

                let prop = Expr::Property(Box::new(expr), name);

                // If optional call or next token is '(', parse call immediately
                if self.check_symbol(Symbol::LParen)? {
                    self.consume()?; // consume '('
                    let (args, forward_rest) = self.parse_argument_list()?;
                    self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
                    expr = Expr::Call {
                        callee: Box::new(prop),
                        args,
                        is_optional,
                        forward_rest,
                    };
                } else {
                    if is_optional {
                        return Err(ParseError::new(
                            "'~' requires a method call: expected '(' after method name"
                                .to_string(),
                            token.line,
                            token.column,
                        ));
                    }
                    expr = prop;
                }
            } else if let Some(token) = self.consume_if_symbol(Symbol::PlusPlus)? {
                self.validate_lvalue(&expr, &token)?;
                expr = Expr::PostIncrement(Box::new(expr));
            } else if let Some(token) = self.consume_if_symbol(Symbol::MinusMinus)? {
                self.validate_lvalue(&expr, &token)?;
                expr = Expr::PostDecrement(Box::new(expr));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_argument_list(&mut self) -> Result<(Vec<Expr>, bool), ParseError> {
        let mut args = Vec::new();
        let mut forward_rest = false;

        if self.check_symbol(Symbol::RParen)? {
            return Ok((args, forward_rest));
        }

        loop {
            // Check for ... (ellipsis) to forward remaining arguments
            if let Some(ellipsis_token) = self.consume_if_symbol(Symbol::Ellipsis)? {
                forward_rest = true;
                // Ellipsis must be the last argument
                if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                    return Err(ParseError::new(
                        "ellipsis (...) must be the last argument in a function call",
                        ellipsis_token.line,
                        ellipsis_token.column,
                    ));
                }
                break;
            }

            // Check for empty argument (comma or rparen immediately following)
            // Empty arguments are represented as nil literals
            if self.check_symbol(Symbol::Comma)? || self.check_symbol(Symbol::RParen)? {
                args.push(Expr::Literal(Literal::Nil));
            } else {
                // Parse regular expression argument
                // Use parse_assignment() instead of parse_expression() to avoid comma operator
                // In argument lists, commas separate arguments, not comma expressions
                args.push(self.parse_assignment()?);
            }

            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }

        Ok((args, forward_rest))
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.consume()?;
        match token.kind {
            TokenKind::Number(value) => Ok(Expr::Literal(Literal::Int(value))),
            TokenKind::String(value) => Ok(Expr::Literal(Literal::String(value))),
            TokenKind::C4Id(id) => Ok(Expr::Literal(Literal::C4Id(id))),
            TokenKind::Keyword(Keyword::True) => Ok(Expr::Literal(Literal::Bool(true))),
            TokenKind::Keyword(Keyword::False) => Ok(Expr::Literal(Literal::Bool(false))),
            TokenKind::Keyword(Keyword::Nil) => Ok(Expr::Literal(Literal::Nil)),
            TokenKind::Keyword(Keyword::This) => {
                // Handle both `this` and `this()` (legacy form)
                if self.check_symbol(Symbol::LParen)? {
                    self.consume()?; // consume '('
                                     // Must be empty argument list
                    if !self.check_symbol(Symbol::RParen)? {
                        return Err(ParseError::new(
                            "this does not accept arguments".to_string(),
                            token.line,
                            token.column,
                        ));
                    }
                    self.consume()?; // consume ')'
                }
                Ok(Expr::This)
            }
            TokenKind::Identifier(name) => Ok(Expr::Variable(name)),
            // Contextual keywords: declaration words carry no expression
            // meaning, so in expression position they are ordinary variable
            // references (the C++ tokenizer emits plain identifiers) —
            // `isPrivate = private;` (Hazard Teleporter.c4d).
            TokenKind::Keyword(
                keyword @ (Keyword::Global
                | Keyword::Private
                | Keyword::Protected
                | Keyword::Public
                | Keyword::Local
                | Keyword::Var
                | Keyword::Static
                | Keyword::Const
                | Keyword::In),
            ) => Ok(Expr::Variable(keyword.lexeme().to_string())),
            TokenKind::Symbol(Symbol::LParen) => {
                let expr = self.parse_expression()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after expression")?;
                Ok(expr)
            }
            TokenKind::Symbol(Symbol::LBracket) => self.parse_array_literal(),
            TokenKind::Symbol(Symbol::LBrace) => self.parse_proplist_literal(),
            _ => Err(ParseError::new(
                "unexpected token in expression",
                token.line,
                token.column,
            )),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, ParseError> {
        if self.consume_if_symbol(Symbol::RBracket)?.is_some() {
            return Ok(Expr::Array(Vec::new()));
        }
        let mut elements = Vec::new();
        loop {
            // Use parse_assignment() instead of parse_expression() to avoid comma operator
            // In arrays, commas separate elements, not comma expressions
            elements.push(self.parse_assignment()?);
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                if self.consume_if_symbol(Symbol::RBracket)?.is_some() {
                    break;
                }
                continue;
            }
            self.expect_symbol(Symbol::RBracket, "expected ']' after array literal")?;
            break;
        }
        Ok(Expr::Array(elements))
    }

    fn parse_proplist_literal(&mut self) -> Result<Expr, ParseError> {
        if self.consume_if_symbol(Symbol::RBrace)?.is_some() {
            return Ok(Expr::Proplist(Vec::new()));
        }
        let mut entries = Vec::new();
        loop {
            let key = self.parse_proplist_key()?;
            self.expect_symbol(Symbol::Equal, "expected '=' after proplist key")?;
            // Use parse_assignment() instead of parse_expression() to avoid comma operator
            // In proplists, commas separate entries, not comma expressions
            let value = self.parse_assignment()?;
            entries.push((key, value));

            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                if self.consume_if_symbol(Symbol::RBrace)?.is_some() {
                    break;
                }
                continue;
            }

            self.expect_symbol(Symbol::RBrace, "expected '}' after proplist literal")?;
            break;
        }
        Ok(Expr::Proplist(entries))
    }

    fn parse_proplist_key(&mut self) -> Result<String, ParseError> {
        let token = self.consume()?;
        match token.kind {
            TokenKind::Identifier(name) => Ok(name),
            TokenKind::String(value) => Ok(value),
            _ => Err(ParseError::new(
                "expected identifier or string for proplist key",
                token.line,
                token.column,
            )),
        }
    }

    fn consume_if_keyword(&mut self, keyword: Keyword) -> Result<Option<Token>, ParseError> {
        if self.check_keyword(keyword)? {
            Ok(Some(self.consume()?))
        } else {
            Ok(None)
        }
    }

    fn consume_if_symbol(&mut self, symbol: Symbol) -> Result<Option<Token>, ParseError> {
        if self.check_symbol(symbol)? {
            Ok(Some(self.consume()?))
        } else {
            Ok(None)
        }
    }

    /// Check if next token is an identifier matching the given name (case-insensitive for operator keywords)
    fn consume_if_identifier(&mut self, name: &str) -> Result<Option<Token>, ParseError> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Identifier(id) if id == name => Ok(Some(self.consume()?)),
            _ => Ok(None),
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword, message: &str) -> Result<(), ParseError> {
        let token = self.peek()?.clone();
        match token.kind {
            TokenKind::Keyword(k) if k == keyword => {
                self.consume()?;
                Ok(())
            }
            _ => Err(ParseError::new(
                message.to_string(),
                token.line,
                token.column,
            )),
        }
    }

    fn expect_symbol(&mut self, symbol: Symbol, message: &str) -> Result<(), ParseError> {
        let token = self.peek()?.clone();
        match token.kind {
            TokenKind::Symbol(sym) if sym == symbol => {
                self.consume()?;
                Ok(())
            }
            _ => Err(ParseError::new(
                message.to_string(),
                token.line,
                token.column,
            )),
        }
    }

    fn expect_identifier(&mut self, message: &str) -> Result<(String, Token), ParseError> {
        let token = self.peek()?.clone();
        let name = match &token.kind {
            TokenKind::Identifier(name) => name.clone(),
            // C4Aul keywords are contextual: the C++ tokenizer emits plain
            // ATT_IDTF for every word, so names like `var func, objhgt`
            // (planet/System.c4g/Commits.c:269) are legal.
            TokenKind::Keyword(keyword) => keyword.lexeme().to_string(),
            _ => {
                return Err(ParseError::new(
                    message.to_string(),
                    token.line,
                    token.column,
                ))
            }
        };
        self.consume()?;
        Ok((name, token))
    }

    fn expect_identifier_or_c4id(
        &mut self,
        message: &str,
    ) -> Result<(String, Token), ParseError> {
        let token = self.peek()?.clone();
        let name = match &token.kind {
            TokenKind::Identifier(name) | TokenKind::C4Id(name) => name.clone(),
            TokenKind::Keyword(keyword) => keyword.lexeme().to_string(),
            _ => {
                return Err(ParseError::new(
                    message.to_string(),
                    token.line,
                    token.column,
                ))
            }
        };
        self.consume()?;
        Ok((name, token))
    }

    fn check_keyword(&mut self, keyword: Keyword) -> Result<bool, ParseError> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Keyword(k) if *k == keyword => Ok(true),
            _ => Ok(false),
        }
    }

    fn check_symbol(&mut self, symbol: Symbol) -> Result<bool, ParseError> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Symbol(sym) if *sym == symbol => Ok(true),
            _ => Ok(false),
        }
    }

    fn check_identifier(&mut self) -> Result<bool, ParseError> {
        let token = self.peek()?;
        Ok(matches!(token.kind, TokenKind::Identifier(_)))
    }

    fn is_eof(&mut self) -> Result<bool, ParseError> {
        let token = self.peek()?;
        Ok(matches!(token.kind, TokenKind::Eof))
    }

    fn peek(&mut self) -> Result<&Token, ParseError> {
        if self.peeked.is_none() {
            // Check lookahead buffer first, then lexer
            if !self.lookahead_buffer.is_empty() {
                self.peeked = Some(self.lookahead_buffer.remove(0));
            } else {
                self.peeked = Some(self.lexer.next_token()?);
            }
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    fn consume(&mut self) -> Result<Token, ParseError> {
        // peek() ensures peeked is populated and honors lookahead buffer
        self.peek()?;
        // Now take the peeked token
        let token = self.peeked.take().unwrap();
        self.account_consumed_token(&token);
        // Track tokens if in speculative mode
        if let Some(ref mut tokens) = self.speculative_tokens {
            tokens.push(token.clone());
        }
        Ok(token)
    }

    fn account_consumed_token(&mut self, token: &Token) {
        self.consumed_tokens = self.consumed_tokens.saturating_add(1);
        match token.kind {
            TokenKind::Symbol(Symbol::LBrace) => {
                self.brace_depth = self.brace_depth.saturating_add(1)
            }
            TokenKind::Symbol(Symbol::RBrace) => {
                self.brace_depth = self.brace_depth.saturating_sub(1)
            }
            _ => {}
        }
    }

    fn rewind_consumed_token(&mut self, token: &Token) {
        self.consumed_tokens = self.consumed_tokens.saturating_sub(1);
        match token.kind {
            TokenKind::Symbol(Symbol::LBrace) => {
                self.brace_depth = self.brace_depth.saturating_sub(1)
            }
            TokenKind::Symbol(Symbol::RBrace) => {
                self.brace_depth = self.brace_depth.saturating_add(1)
            }
            _ => {}
        }
    }

    fn discard_one_recovery_token(&mut self) {
        if self.consume().is_err() {
            self.peeked = None;
        }
    }

    fn next(&mut self) -> Result<Token, ParseError> {
        self.consume()
    }

    fn try_parse_directive(&mut self) -> Result<Option<String>, ParseError> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Directive(directive) => {
                let directive_str = directive.clone();
                self.consume()?;
                Ok(Some(directive_str))
            }
            _ => Ok(None),
        }
    }

    fn parse_var_decl_list(&mut self, kind: VarDeclKind) -> Result<Vec<VarDecl>, ParseError> {
        // Parse: name [= expr] (, name [= expr])* ;
        let mut decls = Vec::new();

        loop {
            // Parse variable name
            let (name, name_token) =
                self.expect_identifier("expected variable name in declaration")?;

            // Check for initializer — parsed BELOW the comma level so the
            // declaration-list comma stays with this loop (`static const
            // A = 5, B = 1;` declares TWO constants, Talker.c4d:5-6).
            let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                Some(self.parse_assignment()?)
            } else {
                None
            };

            // static const requires an initializer
            if kind == VarDeclKind::StaticConst && init.is_none() {
                return Err(ParseError::new(
                    "static const declaration requires an initializer",
                    name_token.line,
                    name_token.column,
                ));
            }

            decls.push(VarDecl { kind, name, init });

            // Check for comma (more declarations) or semicolon (end)
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue; // Parse next declaration
            } else {
                break;
            }
        }

        // Expect semicolon
        self.expect_symbol(Symbol::Semicolon, "expected ';' after variable declaration")?;

        Ok(decls)
    }
}

#[cfg(test)]
mod tests {
#[test]
fn static_const_multi_declarators_parse() {
    // Talker.c4d:5-6: static const _TLK_ID = _TLK,\n _TLK_TimerInterval = 1;
    let source = "#strict\n\nstatic const    _TLK_ID                  = _TLK,\n        _TLK_TimerInterval   = 1;\n";
    let script = crate::Script::compile(source).expect("compiles");
    let names: Vec<&str> = script
        .var_decls()
        .iter()
        .map(|decl| decl.name.as_str())
        .collect();
    assert_eq!(names, vec!["_TLK_ID", "_TLK_TimerInterval"]);
}

    use super::*;

    fn parse_script(source: &str) -> Result<Script, ParseError> {
        Parser::new(source).parse_script()
    }

    // C4Aul precedence: unary `!` binds its operand only — `!A && B` is
    // `(!A) && B`. The speculative assignment-operand parse (`!x = y` ->
    // `!(x = y)`, the DYNB pattern) must not swallow binary chains: the
    // Cowboy Riding guard `!(target->~IsStill()) && GetAction() eq "X"`
    // misparsed as `!(IsStill() && GetAction() eq "X")` fires SetAction
    // without ever evaluating GetAction (the rider 1425 recursion).
    #[test]
    fn bang_binds_unary_operand_not_the_and_chain() {
        let script =
            parse_script(r#"func Test() { return !First() && Second(); }"#).expect("parses");
        let function = &script.functions[0];
        let Stmt::Return(Some(expr)) = &function.body[0] else {
            panic!("expected return with expression");
        };
        match expr {
            Expr::Binary(lhs, BinaryOp::And, _) => {
                assert!(
                    matches!(&**lhs, Expr::Unary(UnaryOp::Not, _)),
                    "lhs must be the negated call, got {lhs:?}"
                );
            }
            other => panic!("expected top-level && with (!First()) lhs, got {other:?}"),
        }
    }

    // The DYNB pattern keeps its special parse: `!x = y` -> `!(x = y)`.
    #[test]
    fn bang_assignment_still_parses_as_negated_assignment() {
        let script = parse_script(r#"func Test() { var x; return !x = 42; }"#).expect("parses");
        let function = &script.functions[0];
        let Stmt::Return(Some(expr)) = &function.body[1] else {
            panic!("expected return with expression");
        };
        assert!(
            matches!(expr, Expr::Unary(UnaryOp::Not, inner) if matches!(&**inner, Expr::Assignment(..))),
            "expected !(x = 42), got {expr:?}"
        );
    }

    #[test]
    fn parse_return_with_simple_expression() {
        let result = parse_script("func Test() { return 42; }");
        assert!(result.is_ok());
    }

    #[test]
    fn function_description_accepts_raw_localized_text() {
        // C4AulParse.cpp:1825-1853 raw-scans the bracket block immediately
        // after `{`; localized descriptions are not C4Script token streams.
        let script = parse_script(
            "func Test() { [Put/Get object | Image=CLNK | Options=[fast/slow] @ menu] return 42; }",
        )
        .expect("raw function description parses");

        assert!(matches!(
            script.functions[0].body.as_slice(),
            [Stmt::Return(Some(Expr::Literal(Literal::Int(42))))]
        ));
        assert_eq!(
            script.functions[0].description.as_deref(),
            Some("Put/Get object | Image=CLNK | Options=[fast/slow] @ menu"),
            "C4Aul function metadata must retain the localized menu description"
        );
    }

    #[test]
    fn function_description_must_close_its_outer_bracket() {
        // C4AulParse.cpp:1832-1844 counts nested brackets and rejects EOF
        // before the outer description bracket closes.
        let error = parse_script("func Test() { [Put/Get [fast/slow] return 42; }")
            .expect_err("unterminated function description must fail");

        assert_eq!(error.message(), "function desc not closed");
    }

    #[test]
    fn old_style_function_description_is_raw_text_too() {
        // C4AulParse.cpp:1757-1759 invokes the same Parse_Desc path directly
        // after an old-style function's colon.
        let script = parse_script("Test: [Put/Get | Options=[fast/slow] @ menu] return 42;")
            .expect("old-style raw function description parses");

        assert!(matches!(
            script.functions[0].body.as_slice(),
            [Stmt::Return(Some(Expr::Literal(Literal::Int(42))))]
        ));
        assert_eq!(
            script.functions[0].description.as_deref(),
            Some("Put/Get | Options=[fast/slow] @ menu")
        );
    }

    #[test]
    fn array_statement_after_first_body_position_is_not_a_description() {
        // C4AulParse.cpp:1709-1711 calls Parse_Desc exactly once, before
        // Parse_Function; a later bracket expression remains executable.
        let script = parse_script("func Test() { var marker; [marker]; return 42; }")
            .expect("array statement parses");

        assert!(script.functions[0]
            .body
            .iter()
            .any(|stmt| matches!(stmt, Stmt::Expr(Expr::Array(values)) if values.len() == 1)));
    }

    #[test]
    fn parse_return_with_parenthesized_expression() {
        let result = parse_script("func Test() { return (42); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_parenthesized_expression_and_operator() {
        let result = parse_script("func Test() { return 100/10; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_complex_expression() {
        let result = parse_script("func Test() { return 255*GetValue()/100; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_without_value() {
        let result = parse_script("func Test() { return; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_empty_parentheses() {
        let result = parse_script("func Test() { return(); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_empty_parens_in_if() {
        let result = parse_script("func Test() { if (1) return(); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_private_function() {
        let result = parse_script("private func Helper() { return 1; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.functions.len(), 1);
        assert_eq!(script.functions[0].access, AccessLevel::Private);
    }

    #[test]
    fn parse_protected_function() {
        let result = parse_script("protected func Helper() { return 1; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.functions[0].access, AccessLevel::Protected);
    }

    #[test]
    fn parse_public_function() {
        let result = parse_script("public func Helper() { return 1; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.functions[0].access, AccessLevel::Public);
    }

    #[test]
    fn parse_global_function() {
        let result = parse_script("global func Helper() { return 1; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.functions[0].access, AccessLevel::Global);
    }

    #[test]
    fn parse_function_defaults_to_public() {
        let result = parse_script("func Helper() { return 1; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.functions[0].access, AccessLevel::Public);
    }

    #[test]
    fn duplicate_parameter_names_reuse_the_c4value_map_slot() {
        let script = parse_script(
            "func Merge(target, number, name, target, timer, change) { return change; }",
        )
        .expect("legacy duplicate parameter names parse");

        assert_eq!(
            script.functions[0]
                .params
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            vec!["target", "number", "name", "timer", "change"],
            "C4ValueMapNames::AddName deduplicates the repeated target slot"
        );
    }

    #[test]
    fn parse_array_index_assignment() {
        let result = parse_script("func Test() { var arr = [1, 2]; arr[0] = 3; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_nested_array_index_assignment() {
        let result = parse_script("func Test() { var m = [[1]]; m[0][0] = 2; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_proplist_property_assignment() {
        let result = parse_script("func Test() { var obj = {}; obj.prop = 1; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_nested_proplist_assignment() {
        let result = parse_script("func Test() { var obj = {n={}}; obj.n.prop = 1; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_empty_statement() {
        let result = parse_script("func Test() { ; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_double_semicolon() {
        let result = parse_script("func Test() { var x = 1;; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_triple_semicolon() {
        let result = parse_script("func Test() { var x = 1;;; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_empty_statement_in_if_without_braces() {
        let result = parse_script("func Test() { if (1) x = 2;; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_comma_single() {
        // Note: return(42); without space is no longer supported
        // Use return (42); or return 42; instead
        let result = parse_script("func Test() { return (42); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_parenthesized_subexpression_and_operator() {
        // Test LENS case: return (expr) op expr;
        let result = parse_script("func Test() { return (255*GetIntensity())/100; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_comma_two_expressions() {
        let result = parse_script("func Test() { return(1, 2); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_comma_three_expressions() {
        let result = parse_script("func Test() { return(1, 2, 3); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_return_with_comma_function_calls() {
        let result = parse_script("func Test() { return(0, Message(), RemoveObject()); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_chained_assignment_simple() {
        let result = parse_script("func Test() { var a = b = 5; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_chained_assignment_triple() {
        let result = parse_script("func Test() { var a = b = c = 10; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_chained_assignment_with_call() {
        // MGDW case: var x = EffectVar(...) = GetValue();
        let result = parse_script("func Test() { var x = EffectVar(0, target) = GetValue(); }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_assignment_in_expression_context() {
        let result = parse_script("func Test() { SetValue(x = 42); }");
        assert!(result.is_ok());
    }
}
