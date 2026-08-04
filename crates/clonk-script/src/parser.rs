use crate::ast::{
    AccessLevel, AppendTo, AssignmentTarget, BinaryOp, Expr, Function, IndexOperand,
    NavigationOperation, Parameter, SafeNavigationStep, Script, Stmt, TypeAnnotation, UnaryOp,
    VarDecl, VarDeclKind,
};
use crate::error::ParseError;
use crate::lexer::Lexer;
use crate::token::{Keyword, Symbol, Token, TokenKind};
use crate::value::Literal;

/// `C4AUL_MAX_Par`: a new-style function declaration has ten syntactic
/// parameter slots at most (C4Aul.h; C4AulParse.cpp:1624-1640).
const MAX_FUNCTION_PARAMETERS: usize = 10;

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
    /// Script-wide object-local/static declarations discovered at any lexical
    /// depth. C4Aul's preparser registers these without emitting bytecode.
    script_var_decls: Vec<VarDecl>,
    /// System/global scripts have no definition owner. Their legacy
    /// old-style `local` declarations produce a preparser diagnostic.
    global_script: bool,
    parsing_old_style_function: bool,
    /// Preparser-only diagnostics that do not poison the retained function.
    /// The recovering top-level loop drains these in source order.
    non_fatal_diagnostics: Vec<ParseError>,
    /// Active loop bodies in the current function. C4Aul only emits control
    /// flow for `break`/`continue` while a loop parse context exists.
    loop_depth: usize,
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
            script_var_decls: Vec::new(),
            global_script: false,
            parsing_old_style_function: false,
            non_fatal_diagnostics: Vec::new(),
            loop_depth: 0,
        }
    }

    pub(crate) fn new_global_script(source: &'a str) -> Self {
        let mut parser = Self::new(source);
        parser.global_script = true;
        parser
    }

    pub(crate) fn new_global_script_c4_string(source: &'a str) -> Self {
        let mut parser = Self::new(source);
        parser.lexer = Lexer::new_c4_string(source);
        parser.global_script = true;
        parser
    }

    #[cfg(test)]
    pub(crate) fn with_strict_level(source: &'a str, strict_level: Option<u8>) -> Self {
        let mut parser = Self::new(source);
        parser.strict_level = strict_level.unwrap_or(0);
        parser.lexer.set_strict_level(parser.strict_level);
        parser
    }

    pub(crate) fn with_strict_level_c4_string(source: &'a str, strict_level: Option<u8>) -> Self {
        let mut parser = Self::new(source);
        parser.lexer = Lexer::new_c4_string(source);
        parser.strict_level = strict_level.unwrap_or(0);
        parser.lexer.set_strict_level(parser.strict_level);
        parser
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
        let mut functions = Vec::new();

        while !self.is_eof()? {
            let declaration_token = self.peek()?.clone();
            // Check for directives
            if let Some(directive) = self.try_parse_directive()? {
                self.parse_script_directive(
                    &directive,
                    declaration_token.line,
                    declaration_token.column,
                    &mut includes,
                    &mut appends,
                    &mut strict_level,
                )?;
            } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Local) {
                // Parse local variable declarations
                self.consume()?; // consume 'local'
                self.parse_var_decl_list(VarDeclKind::Local)?;
            } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Static) {
                // Parse static variable declarations
                self.consume()?; // consume 'static'
                                 // Check for 'const' after 'static'
                if self.consume_if_keyword(Keyword::Const)?.is_some() {
                    self.parse_var_decl_list(VarDeclKind::StaticConst)?;
                } else {
                    self.parse_var_decl_list(VarDeclKind::Static)?;
                }
            } else {
                // Must be a function
                functions.push(self.parse_function()?);
            }
        }

        Ok(Script::with_directives(
            functions,
            std::mem::take(&mut self.script_var_decls),
            self.lexer.take_string_literals(),
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
                    self.parse_script_directive(
                        &directive,
                        declaration_token.line,
                        declaration_token.column,
                        &mut includes,
                        &mut appends,
                        &mut strict_level,
                    )?;
                } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Local) {
                    self.consume()?;
                    self.parse_var_decl_list(VarDeclKind::Local)?;
                } else if self.peek()?.kind == TokenKind::Keyword(Keyword::Static) {
                    self.consume()?;
                    if self.consume_if_keyword(Keyword::Const)?.is_some() {
                        self.parse_var_decl_list(VarDeclKind::StaticConst)?;
                    } else {
                        self.parse_var_decl_list(VarDeclKind::Static)?;
                    }
                } else {
                    let (function, error) = self.parse_function_recovering()?;
                    functions.push(function);
                    diagnostics.append(&mut self.non_fatal_diagnostics);
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

        diagnostics.append(&mut self.non_fatal_diagnostics);
        diagnostics.extend(self.lexer.take_diagnostics());

        (
            Script::with_directives(
                functions,
                std::mem::take(&mut self.script_var_decls),
                self.lexer.take_string_literals(),
                includes,
                appends,
                strict_level,
            ),
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
        let (name, name_token) = self.expect_identifier("expected function name")?;
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
            // C4Aul's SGetLine counts preceding newlines, while lexer
            // diagnostics use conventional one-based lines.
            source_line: name_token.line.saturating_sub(1),
            source_host: None,
            source_name: None,
            // Bound to the destination ScriptEngine when the script is added.
            global_link_host: None,
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
        let (name, name_token) = self.expect_identifier("expected function name")?;
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
                source_line: name_token.line.saturating_sub(1),
                source_host: None,
                source_name: None,
                global_link_host: None,
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

        let previous_old_style = self.parsing_old_style_function;
        self.parsing_old_style_function = true;
        let body = (|| {
            let mut body = Vec::new();
            while !self.is_eof()? && !self.is_old_style_function_boundary()? {
                body.push(self.parse_statement()?);
            }
            Ok(body)
        })();
        self.parsing_old_style_function = previous_old_style;
        let body = body?;

        Ok(Function {
            name,
            params: Vec::new(),
            body,
            access,
            returns_reference: false,
            description,
            strict_level: None,
            source_line: name_token.line.saturating_sub(1),
            source_host: None,
            source_name: None,
            global_link_host: None,
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

        let previous_old_style = self.parsing_old_style_function;
        self.parsing_old_style_function = true;
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
        self.parsing_old_style_function = previous_old_style;

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
                source_line: name_token.line.saturating_sub(1),
                source_host: None,
                source_name: None,
                global_link_host: None,
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
            TokenKind::Identifier(_) | TokenKind::C4Id(_) => {}
            TokenKind::Keyword(keyword) if *keyword != Keyword::Nil => {}
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
        // C++ advances `cpar` for every comma-delimited declaration even
        // when C4ValueMapNames::AddName deduplicates its name. Do not derive
        // this limit from `params.len()`.
        let mut syntactic_parameter_count = 0;
        if self.check_symbol(Symbol::RParen)? {
            return Ok(params);
        }
        loop {
            // C++ checks ')' before the cap on every iteration. Besides the
            // empty list, this admits its legacy trailing comma after the
            // tenth parameter.
            if self.check_symbol(Symbol::RParen)? {
                break;
            }

            // The token after `(` or `,` is already scanned before C++ checks
            // the ten-parameter cap. Invalid operator-disabled bytes therefore
            // win over the cap diagnostic.
            self.reject_initial_parameter_disabled_operator()?;

            // The cap check also precedes `...`: nine named parameters plus
            // ellipsis is legal, but ten named parameters plus ellipsis is
            // the rejected eleventh iteration.
            if syntactic_parameter_count >= MAX_FUNCTION_PARAMETERS {
                let token = self.peek()?.clone();
                return Err(ParseError::new(
                    "'func' parameter list: too many parameters (max 10)",
                    token.line,
                    token.column,
                ));
            }

            // `...` ends the parameter list: the function takes anything via
            // Par() and declares no further names (C4AulParse.cpp:1642-1648).
            if self.consume_if_symbol(Symbol::Ellipsis)?.is_some() {
                break;
            }

            // Strict-3 `nil` is ATT_NIL, not an identifier or a parameter
            // type.
            let parameter_start = self.peek()?.clone();
            if matches!(parameter_start.kind, TokenKind::Keyword(Keyword::Nil)) {
                return Err(ParseError::new(
                    "expected parameter name",
                    parameter_start.line,
                    parameter_start.column,
                ));
            }

            // C++ tokenizes parameter heads with operators disabled. In
            // legacy modes, a single `|` can therefore begin a wacky ATT_IDTF
            // such as `|nil`; STRICT2+ rejects the same byte. Preserve that
            // distinction without admitting Rust's former union grammar.
            let leading_legacy_name = self.consume_legacy_pipe_parameter_name()?;
            let (type_annotation, is_reference, legacy_name) =
                if let Some(name) = leading_legacy_name {
                    (None, false, Some(name))
                } else {
                    let type_annotation = self.parse_type_annotation()?;
                    self.reject_parameter_disabled_operator()?;
                    if let Some(name) = self.consume_legacy_pipe_parameter_name()? {
                        (type_annotation, false, Some(name))
                    } else {
                        let is_reference = self.consume_if_symbol(Symbol::Ampersand)?.is_some();
                        self.reject_parameter_disabled_operator()?;
                        let legacy_name = self.consume_legacy_pipe_parameter_name()?;
                        (type_annotation, is_reference, legacy_name)
                    }
                };

            // Try to get the parameter name. C++ warns below STRICT2 when a
            // recognized type word is the declaration's only word, then
            // binds it as an untyped parameter. STRICT2+ rejects it.
            let next_token = self.peek()?.clone();
            let (name, actual_type, actual_is_reference) = if let Some(name) = legacy_name {
                (name, type_annotation, is_reference)
            } else {
                match &next_token.kind {
                    TokenKind::Identifier(param_name) => {
                        let name = param_name.clone();
                        self.consume()?;
                        (name, type_annotation, is_reference)
                    }
                    // C4Aul declaration words are contextual (plain ATT_IDTF
                    // words in the C++ tokenizer), so names such as `private`
                    // remain legal. Boolean literals and strict-3 `nil` are
                    // distinct native tokens and cannot be parameter names.
                    TokenKind::Keyword(keyword)
                        if !matches!(*keyword, Keyword::Nil | Keyword::True | Keyword::False) =>
                    {
                        let name = keyword.lexeme().to_string();
                        self.consume()?;
                        (name, type_annotation, is_reference)
                    }
                    _ => {
                        let Some(annotation) = type_annotation else {
                            return Err(ParseError::new(
                                "expected parameter name",
                                next_token.line,
                                next_token.column,
                            ));
                        };
                        let param_name = annotation.to_string();
                        let diagnostic = ParseError::new(
                            format!("parameter has the same name as type {param_name}"),
                            next_token.line,
                            next_token.column,
                        );
                        if self.strict_level >= 2 {
                            return Err(diagnostic);
                        }
                        self.non_fatal_diagnostics.push(diagnostic);
                        // C4AulParse.cpp resets ParType to Any after this warning,
                        // including when an ampersand followed the type word.
                        (param_name, None, false)
                    }
                }
            };

            // C4Aul stores parameter names in C4ValueMapNames. AddName
            // returns the existing slot for a duplicate instead of adding a
            // new positional name (C4AulParse.cpp:1677-1682;
            // C4ValueMap.cpp:406-411). A few shipped legacy callbacks rely
            // on the following unique name shifting into that reused slot.
            if !params.iter().any(|parameter| parameter.name == name) {
                if actual_is_reference || actual_type.is_some() {
                    params.push(Parameter::with_reference(
                        name,
                        actual_type,
                        actual_is_reference,
                    ));
                } else {
                    params.push(Parameter::new(name));
                }
            }
            syntactic_parameter_count += 1;

            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }
        Ok(params)
    }

    fn reject_parameter_disabled_operator(&mut self) -> Result<(), ParseError> {
        let token = self.peek()?.clone();
        let invalid = match token.kind {
            TokenKind::Symbol(Symbol::Pipe | Symbol::OrOr | Symbol::OrEqual)
                if self.strict_level >= 2 =>
            {
                Some(('|', 0))
            }
            // With operators disabled, C++ scans `&=` as ATT_AMP followed by
            // an invalid `=` byte rather than as one compound operator.
            TokenKind::Symbol(Symbol::AndEqual) => Some(('=', 1)),
            TokenKind::Symbol(Symbol::Equal | Symbol::EqualEqual) => Some(('=', 0)),
            _ => None,
        };
        let Some((invalid, column_offset)) = invalid else {
            return Ok(());
        };
        self.consume()?;
        Err(ParseError::new(
            format!("unexpected character '{invalid}' found"),
            token.line,
            token.column + column_offset,
        ))
    }

    fn reject_initial_parameter_disabled_operator(&mut self) -> Result<(), ParseError> {
        let token = self.peek()?.clone();
        let invalid = match token.kind {
            TokenKind::Symbol(Symbol::Pipe | Symbol::OrOr | Symbol::OrEqual)
                if self.strict_level >= 2 =>
            {
                Some('|')
            }
            TokenKind::Symbol(Symbol::Equal | Symbol::EqualEqual) => Some('='),
            _ => None,
        };
        let Some(invalid) = invalid else {
            return Ok(());
        };
        self.consume()?;
        Err(ParseError::new(
            format!("unexpected character '{invalid}' found"),
            token.line,
            token.column,
        ))
    }

    fn consume_legacy_pipe_parameter_name(&mut self) -> Result<Option<String>, ParseError> {
        if self.strict_level >= 2 {
            return Ok(None);
        }
        let pipe = self.peek()?.clone();
        if !matches!(pipe.kind, TokenKind::Symbol(Symbol::Pipe)) {
            return Ok(None);
        }
        self.consume()?;

        let mut name = String::from("|");
        debug_assert!(self.peeked.is_none() && self.lookahead_buffer.is_empty());
        name.push_str(&self.lexer.consume_legacy_identifier_continuation());
        Ok(Some(name))
    }

    fn parse_type_annotation(&mut self) -> Result<Option<TypeAnnotation>, ParseError> {
        let annotation = match &self.peek()?.kind {
            // C4Aul's eight type words are case-sensitive contextual
            // identifiers (C4AulParse.cpp:1654-1667).
            TokenKind::Identifier(name) => match name.as_str() {
                "int" => TypeAnnotation::Int,
                "bool" => TypeAnnotation::Bool,
                "string" => TypeAnnotation::String,
                "object" => TypeAnnotation::Object,
                "id" => TypeAnnotation::Id,
                "array" => TypeAnnotation::Array,
                "map" => TypeAnnotation::Map,
                "any" => TypeAnnotation::Any,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        self.consume()?;
        Ok(Some(annotation))
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

    fn parse_loop_body(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.loop_depth += 1;
        let body = self.parse_stmt_or_block_vec();
        self.loop_depth -= 1;
        body
    }

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        if self.consume_if_keyword(Keyword::Var)?.is_some() {
            return self.parse_var_decl();
        }
        if let Some(token) = self.consume_if_keyword(Keyword::Local)? {
            if self.global_script && self.parsing_old_style_function {
                self.non_fatal_diagnostics.push(ParseError::new(
                    "'local' variable declaration in global script",
                    token.line,
                    token.column,
                ));
                // C++ reports this in the PREPARSER, then recovery retries
                // the same token as a top-level declaration and registers
                // its names. The later PARSER pass still compiles the rest of
                // the retained function.
                self.parse_var_decl_list(VarDeclKind::Local)?;
                return Ok(Stmt::Sequence(Vec::new()));
            }
            self.parse_var_decl_list(VarDeclKind::Local)?;
            return Ok(Stmt::Sequence(Vec::new()));
        }
        if self.consume_if_keyword(Keyword::Static)?.is_some() {
            self.parse_var_decl_list(VarDeclKind::Static)?;
            return Ok(Stmt::Sequence(Vec::new()));
        }
        if self.consume_if_keyword(Keyword::Return)?.is_some() {
            return self.parse_return();
        }
        if let Some(token) = self.consume_if_keyword(Keyword::Break)? {
            if self.loop_depth == 0 {
                let error = ParseError::new(
                    "'break' is only allowed inside loops",
                    token.line,
                    token.column,
                );
                if self.strict_level >= 2 {
                    return Err(error);
                }
                self.non_fatal_diagnostics.push(error);
                self.expect_symbol(Symbol::Semicolon, "expected ';' after break")?;
                return Ok(Stmt::Sequence(Vec::new()));
            }
            self.expect_symbol(Symbol::Semicolon, "expected ';' after break")?;
            return Ok(Stmt::Break);
        }
        if let Some(token) = self.consume_if_keyword(Keyword::Continue)? {
            if self.loop_depth == 0 {
                let error = ParseError::new(
                    "'continue' is only allowed inside loops",
                    token.line,
                    token.column,
                );
                if self.strict_level >= 2 {
                    return Err(error);
                }
                self.non_fatal_diagnostics.push(error);
                self.expect_symbol(Symbol::Semicolon, "expected ';' after continue")?;
                return Ok(Stmt::Sequence(Vec::new()));
            }
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
        // C4Aul cannot decide from the opening brace alone whether this is a
        // statement block or a STRICT3 map-literal expression. Probe the
        // outer brace contents exactly like IsMapLiteral: nested delimiter
        // groups do not participate, a top-level `=` selects a map, and a
        // top-level semicolon selects a block even if an `=` preceded it.
        // Empty/uncertain braces remain blocks.
        if self.check_symbol(Symbol::LBrace)? && self.statement_brace_is_map_literal()? {
            return self.parse_assignment_or_expr();
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

    /// C4AulParse.cpp `IsMapLiteral` scans from just after a statement-leading
    /// `{` through its matching outer `}`. It skips complete nested `()`, `[]`
    /// and `{}` groups, remembers an outer-level plain `=`, and treats an
    /// outer-level `;` as conclusive block syntax. C++ performs this scan with
    /// its `Discard` string policy and then rewinds the source pointer. Restore
    /// the complete token cursor (rather than replaying already-lexed tokens)
    /// so quoted strings are committed only by the real parse that follows.
    /// Lexer warnings deliberately remain, matching C++ lookahead side
    /// effects (and may be emitted again by the real parse).
    fn statement_brace_is_map_literal(&mut self) -> Result<bool, ParseError> {
        assert!(
            self.speculative_tokens.is_none(),
            "statement lookahead cannot nest inside speculative parsing"
        );
        let lexer_checkpoint = self.lexer.checkpoint();
        let peeked = self.peeked.clone();
        let lookahead_buffer = self.lookahead_buffer.clone();
        let brace_depth = self.brace_depth;
        let consumed_tokens = self.consumed_tokens;

        let result = (|| {
            self.expect_symbol(Symbol::LBrace, "expected '{' to probe statement")?;
            let mut is_map = false;

            loop {
                match self.consume()?.kind {
                    TokenKind::Symbol(Symbol::LParen) => {
                        self.skip_statement_probe_group(Symbol::RParen)?
                    }
                    TokenKind::Symbol(Symbol::LBracket) => {
                        self.skip_statement_probe_group(Symbol::RBracket)?
                    }
                    TokenKind::Symbol(Symbol::LBrace) => {
                        self.skip_statement_probe_group(Symbol::RBrace)?
                    }
                    TokenKind::Symbol(Symbol::Equal) => is_map = true,
                    TokenKind::Symbol(Symbol::Semicolon) => return Ok(false),
                    TokenKind::Symbol(Symbol::RBrace) | TokenKind::Eof => return Ok(is_map),
                    _ => {}
                }
            }
        })();

        if result.is_ok() {
            self.lexer.restore(lexer_checkpoint);
            self.peeked = peeked;
            self.lookahead_buffer = lookahead_buffer;
            self.brace_depth = brace_depth;
            self.consumed_tokens = consumed_tokens;
        } else {
            // An exception bypasses C++ IsMapLiteral's `SPos = SPos0`.
            // Preserve that forward progress for recovery, but still honor
            // the lookahead's Discard policy for strings scanned before it.
            self.lexer.finish_failed_discard_scan(lexer_checkpoint);
        }
        result
    }

    /// `SkipBlock<closingAtt>` from the C++ lookahead: nested groups recurse,
    /// mismatched closing tokens are ordinary contents, and EOF terminates the
    /// current skip without manufacturing a parse error.
    fn skip_statement_probe_group(&mut self, closing: Symbol) -> Result<(), ParseError> {
        loop {
            match self.consume()?.kind {
                TokenKind::Symbol(symbol) if symbol == closing => return Ok(()),
                TokenKind::Eof => return Ok(()),
                TokenKind::Symbol(Symbol::LParen) => {
                    self.skip_statement_probe_group(Symbol::RParen)?
                }
                TokenKind::Symbol(Symbol::LBracket) => {
                    self.skip_statement_probe_group(Symbol::RBracket)?
                }
                TokenKind::Symbol(Symbol::LBrace) => {
                    self.skip_statement_probe_group(Symbol::RBrace)?
                }
                _ => {}
            }
        }
    }

    fn parse_var_decl(&mut self) -> Result<Stmt, ParseError> {
        let mut decls = Vec::new();

        loop {
            // Parse one variable
            let (name, _) = self.expect_identifier("expected variable name")?;
            let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                // Commas in variable declarations separate declarators.
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

            // At this level commas delimit the legacy return parameters.
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
                    IndexOperand::Dynamic(Box::new(Expr::Literal(Literal::Int(0)))),
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
        let condition = self.parse_condition_parameters("if")?;

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
        let condition = self.parse_condition_parameters("while")?;

        // Parse either a single statement or a braced block for the loop body.
        let body = self.parse_loop_body()?;

        Ok(Stmt::While { condition, body })
    }

    fn parse_condition_parameters(&mut self, statement: &str) -> Result<Expr, ParseError> {
        let opening = self.peek()?.clone();
        self.expect_symbol(Symbol::LParen, &format!("expected '(' after '{statement}'"))?;

        if self.strict_level >= 2 {
            let condition = self.parse_expression()?;
            self.expect_symbol(Symbol::RParen, "expected ')' after condition")?;
            return Ok(condition);
        }

        let (args, forward_rest) = self.parse_argument_list()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after condition parameters")?;
        if args.len() > 1 {
            self.non_fatal_diagnostics.push(ParseError::new(
                format!(
                    "{statement}: passing {} parameters, but only 1 are used",
                    args.len()
                ),
                opening.line,
                opening.column,
            ));
        }
        Ok(Expr::LegacyParameterList { args, forward_rest })
    }

    /// Probe the tokens after `for (` without consuming them. C4Aul has two
    /// foreach header shapes: `[var] item in array` and
    /// `[var] key, value in map`. Everything else belongs to the C-style
    /// parser, including declaration lists such as `var i, j;`.
    fn probe_for_in_binder_count(&mut self) -> Result<Option<usize>, ParseError> {
        self.begin_speculative();
        let result = (|| {
            self.consume_if_keyword(Keyword::Var)?;
            if !Self::is_identifier_name_token(&self.peek()?.kind) {
                return Ok(None);
            }
            self.consume()?;

            if self.consume_if_keyword(Keyword::In)?.is_some() {
                return Ok(Some(1));
            }
            if self.consume_if_symbol(Symbol::Comma)?.is_none()
                || !Self::is_identifier_name_token(&self.peek()?.kind)
            {
                return Ok(None);
            }
            self.consume()?;

            Ok(self.consume_if_keyword(Keyword::In)?.map(|_| 2))
        })();
        self.reset_speculative();
        result
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        use crate::ast::ForInit;

        self.expect_symbol(Symbol::LParen, "expected '(' after 'for'")?;

        if let Some(binder_count) = self.probe_for_in_binder_count()? {
            let declare_var = self.consume_if_keyword(Keyword::Var)?.is_some();
            let (variable, _) = self.expect_identifier("expected variable name")?;
            let value_variable = if binder_count == 2 {
                self.expect_symbol(Symbol::Comma, "expected ',' between map variables")?;
                Some(
                    self.expect_identifier("expected map value variable name")?
                        .0,
                )
            } else {
                None
            };
            self.expect_keyword(Keyword::In, "expected 'in' after foreach variables")?;
            let iterable = self.parse_expression()?;
            self.expect_symbol(Symbol::RParen, "expected ')' after for-in header")?;
            let body = self.parse_loop_body()?;

            return Ok(Stmt::ForIn {
                variable,
                value_variable,
                declare_var,
                iterable,
                body,
            });
        }

        // C-style declaration loop: for(var i = 0, j = 1; cond; incr)
        if self.check_keyword(Keyword::Var)? {
            self.consume()?;
            let (variable, _) = self.expect_identifier("expected variable name")?;
            let mut decls = Vec::new();

            // Commas in this clause separate declarations.
            let first_init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                Some(self.parse_assignment()?)
            } else {
                None
            };
            decls.push((variable, first_init));

            while self.consume_if_symbol(Symbol::Comma)?.is_some() {
                let (name, _) = self.expect_identifier("expected variable name")?;
                let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                    Some(self.parse_assignment()?)
                } else {
                    None
                };
                decls.push((name, init));
            }

            self.expect_symbol(Symbol::Semicolon, "expected ';' after for-init")?;
            let condition = if self.check_symbol(Symbol::Semicolon)? {
                None
            } else {
                Some(self.parse_expression()?)
            };
            self.expect_symbol(Symbol::Semicolon, "expected ';' after for-condition")?;
            let increment = if self.check_symbol(Symbol::RParen)? {
                None
            } else {
                Some(self.parse_expression()?)
            };
            self.expect_symbol(Symbol::RParen, "expected ')' after for-clauses")?;
            let body = self.parse_loop_body()?;

            return Ok(Stmt::For {
                init: Some(ForInit::VarDecls(decls)),
                condition,
                increment,
                body,
            });
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
        let body = self.parse_loop_body()?;

        Ok(Stmt::For {
            init,
            condition,
            increment,
            body,
        })
    }

    fn parse_assignment_or_expr(&mut self) -> Result<Stmt, ParseError> {
        let legacy_goto_call = self.probe_leading_goto_call()?;
        let expr = self.parse_expression()?;
        // At statement level, we always expect a trailing semicolon
        self.expect_symbol(Symbol::Semicolon, "expected ';' after expression")?;

        if let Some(call) = legacy_goto_call {
            return Ok(Stmt::LegacyGoto {
                call,
                expression: expr,
            });
        }

        match expr {
            Expr::Assignment(target, value) => Ok(Stmt::Assignment {
                target,
                value: *value,
            }),
            _ => Ok(Stmt::Expr(expr)),
        }
    }

    /// C4AulParse.cpp:2193-2248 recognizes the legacy goto hack from the
    /// statement's first token, then emits AB_RETURN immediately after the
    /// direct call. Probe just that call and replay the tokens so the normal
    /// expression parser still validates any (unreachable) suffix. Starting
    /// with `(`, another call, an assignment, or any other prefix deliberately
    /// stays on the ordinary expression path.
    fn probe_leading_goto_call(&mut self) -> Result<Option<Expr>, ParseError> {
        if !matches!(
            &self.peek()?.kind,
            TokenKind::Identifier(name) if name == "goto"
        ) {
            return Ok(None);
        }

        self.begin_speculative();
        let result = (|| {
            self.consume()?; // `goto`
            if self.consume_if_symbol(Symbol::LParen)?.is_none() {
                return Ok(None);
            }
            let (args, forward_rest) = self.parse_argument_list()?;
            self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
            Ok(Some(Expr::Call {
                callee: Box::new(Expr::Variable("goto".to_string())),
                args,
                is_optional: false,
                forward_rest,
            }))
        })();
        self.reset_speculative();
        // Lexer errors consume their offending bytes and cannot be replayed;
        // propagate the original result instead of silently reparsing after
        // the bad byte (for example, turning strict `goto(@)` into `goto()`).
        result
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
            Expr::ArrayAppend(base) => Ok(AssignmentTarget::ArrayAppend(base)),
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
                                is_arrow: false,
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
                            is_arrow: true,
                        });
                    }
                }
                Err(ParseError::new(
                    "invalid assignment target",
                    eq_token.line,
                    eq_token.column,
                ))
            }
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => Ok(AssignmentTarget::GlobalFunctionCall {
                name,
                args,
                failsafe,
                forward_rest,
            }),
            _ => Err(ParseError::new(
                "invalid assignment target",
                eq_token.line,
                eq_token.column,
            )),
        }
    }

    fn validate_lvalue(&self, expr: &Expr, token: &Token) -> Result<(), ParseError> {
        match expr {
            Expr::Variable(_)
            | Expr::Property(_, _)
            | Expr::Index(_, _)
            | Expr::ArrayAppend(_) => Ok(()),
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
            Expr::GlobalCall { .. } => Ok(()),
            _ => Err(ParseError::new(
                "increment/decrement requires an lvalue (variable, property, index, Local(n), or Var(n))",
                token.line,
                token.column,
            )),
        }
    }

    fn assignment_target_contains_array_append(target: &AssignmentTarget) -> bool {
        match target {
            AssignmentTarget::ArrayAppend(_) => true,
            AssignmentTarget::Property(base, _) | AssignmentTarget::Index(base, _) => {
                Self::assignment_target_contains_array_append(base)
            }
            _ => false,
        }
    }

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_assignment()
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
            TokenKind::Symbol(Symbol::StarStarEqual) => (true, Some(Symbol::StarStarEqual)),
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
            let op_symbol = op_symbol.ok_or_else(|| {
                ParseError::new(
                    "missing assignment operator",
                    op_token.line,
                    op_token.column,
                )
            })?;
            let operator = match op_symbol {
                Symbol::Equal => "=",
                Symbol::PlusEqual => "+=",
                Symbol::MinusEqual => "-=",
                Symbol::StarStarEqual => "**=",
                Symbol::StarEqual => "*=",
                Symbol::SlashEqual => "/=",
                Symbol::PercentEqual => "%=",
                Symbol::AndEqual => "&=",
                Symbol::OrEqual => "|=",
                Symbol::XorEqual => "^=",
                Symbol::LeftShiftEqual => "<<=",
                Symbol::RightShiftEqual => ">>=",
                Symbol::ConcatEqual => "..=",
                Symbol::QuestionQuestionEqual => "??=",
                _ => {
                    return Err(ParseError::new(
                        format!("unknown assignment operator {op_symbol:?}"),
                        op_token.line,
                        op_token.column,
                    ))
                }
            };
            // C4Aul's precedence parser can emit AB_Set after a value result;
            // AB_Set rejects that result as a non-reference at runtime. Keep
            // this narrow to the `!`-led shape whose old speculative parse
            // incorrectly swallowed the assignment into the unary operand.
            let target = match Self::expression_to_assignment_target(left.clone(), &op_token) {
                Ok(target) => target,
                Err(_) if matches!(&left, Expr::Unary(UnaryOp::Not, _)) => {
                    AssignmentTarget::InvalidValue {
                        expression: Box::new(left.clone()),
                        operator,
                    }
                }
                Err(error) => return Err(error),
            };

            // Right-associative: a = b = c parses as a = (b = c)
            let value = self.parse_assignment()?;

            if matches!(&target, AssignmentTarget::InvalidValue { .. }) {
                // The target's reference conversion always fails, so compound
                // arithmetic never runs. Store only the raw RHS to preserve
                // C++'s single left-then-right evaluation before that error.
                return Ok(Expr::Assignment(target, Box::new(value)));
            }

            // Retain one evaluated target reference for compound assignment.
            // Desugaring `a += b` to `a = a + b` evaluates side-effecting
            // index/address expressions twice. `array[]` also needs this node
            // for plain assignment because appending precedes RHS evaluation.
            let operation = match op_symbol {
                Symbol::Equal => None,
                Symbol::PlusEqual => Some(BinaryOp::Add),
                Symbol::MinusEqual => Some(BinaryOp::Sub),
                Symbol::StarStarEqual => Some(BinaryOp::Pow),
                Symbol::StarEqual => Some(BinaryOp::Mul),
                Symbol::SlashEqual => Some(BinaryOp::Div),
                Symbol::PercentEqual => Some(BinaryOp::Mod),
                Symbol::ConcatEqual => Some(BinaryOp::Concat),
                Symbol::QuestionQuestionEqual => Some(BinaryOp::NilCoalescing),
                Symbol::AndEqual => Some(BinaryOp::BitAnd),
                Symbol::OrEqual => Some(BinaryOp::BitOr),
                Symbol::XorEqual => Some(BinaryOp::BitXor),
                Symbol::LeftShiftEqual => Some(BinaryOp::LeftShift),
                Symbol::RightShiftEqual => Some(BinaryOp::RightShift),
                _ => {
                    return Err(ParseError::new(
                        format!("unknown assignment operator {op_symbol:?}"),
                        op_token.line,
                        op_token.column,
                    ))
                }
            };

            if Self::assignment_target_contains_array_append(&target) {
                return Ok(Expr::ArrayAppendAssignment {
                    target,
                    operation,
                    operator,
                    value: Box::new(value),
                });
            }

            match operation {
                Some(operation) => Ok(Expr::CompoundAssignment {
                    target,
                    operation,
                    operator,
                    value: Box::new(value),
                }),
                None => Ok(Expr::Assignment(target, Box::new(value))),
            }
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
        while self.consume_if_symbol(Symbol::OrOr)?.is_some() {
            let right = self.parse_and()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::Or, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_or()?;
        while self.consume_if_symbol(Symbol::AndAnd)?.is_some() {
            let right = self.parse_bit_or()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::And, Box::new(right));
        }
        Ok(expr)
    }

    /// C4Script gives `|` and `^` the same precedence, unlike C/Rust.
    fn parse_bit_or(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_and()?;
        loop {
            let operation = if self.consume_if_symbol(Symbol::Pipe)?.is_some() {
                BinaryOp::BitOr
            } else if self.consume_if_symbol(Symbol::Caret)?.is_some() {
                BinaryOp::BitXor
            } else {
                break;
            };
            let right = self.parse_bit_and()?;
            expr = Expr::Binary(Box::new(expr), operation, Box::new(right));
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
            } else if self.strict_level < 2 && self.consume_if_identifier("eq")?.is_some() {
                let right = self.parse_concat()?;
                expr = Expr::Binary(
                    Box::new(expr),
                    BinaryOp::KeywordStringEqual,
                    Box::new(right),
                );
            } else if self.strict_level < 2 && self.consume_if_identifier("ne")?.is_some() {
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
            if self.consume_if_symbol(Symbol::Less)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Less, Box::new(right));
            } else if self.consume_if_symbol(Symbol::LessEqual)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::LessEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Greater)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Greater, Box::new(right));
            } else if self.consume_if_symbol(Symbol::GreaterEqual)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::GreaterEqual, Box::new(right));
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
        // C4ScriptOpMap marks `**` left-associative: fold a chain as
        // `(2**3)**2`, while each operand still keeps unary precedence.
        while self.consume_if_symbol(Symbol::StarStar)?.is_some() {
            let right = self.parse_unary()?;
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
        if self.consume_if_symbol(Symbol::Bang)?.is_some() {
            // The speculative preflight predates the `(!x) = y` precedence
            // fix and now contributes only a fallback error. A lexer error
            // never becomes a token, so `reset_speculative` has nothing to
            // replay for the text the preflight scanned past — which is why
            // `!Foo(<oversized literal>)` still compiles, recorded under
            // "Open" in PORT_STATUS.md. Its AST must never choose
            // precedence: replay and parse only the unary operand, so
            // `!x = y` stays `(!x) = y`.
            if self.speculative_tokens.is_none() {
                self.begin_speculative();
                let speculative_result = self.parse_assignment();
                self.reset_speculative();
                return match self.parse_unary() {
                    Ok(expr) => Ok(Expr::Unary(UnaryOp::Not, Box::new(expr))),
                    Err(error) => Err(speculative_result.err().unwrap_or(error)),
                };
            }

            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnaryOp::Not, Box::new(expr)));
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
            } else if let Some(question) = self.consume_if_symbol(Symbol::Question)? {
                let steps = self.parse_safe_navigation_steps(&question)?;
                expr = Expr::SafeNavigation {
                    receiver: Box::new(expr),
                    steps,
                };
            } else if let Some(operation) = self.parse_navigation_operation()? {
                expr = Self::apply_navigation_operation(expr, operation);
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

    fn parse_safe_navigation_steps(
        &mut self,
        question: &Token,
    ) -> Result<Vec<SafeNavigationStep>, ParseError> {
        if self.strict_level < 3 {
            return Err(ParseError::new(
                "unexpected '?'".to_string(),
                question.line,
                question.column,
            ));
        }

        let mut steps = Vec::new();
        let mut nil_guard = true;
        loop {
            let Some(operation) = self.parse_navigation_operation()? else {
                if steps.is_empty() || nil_guard {
                    let token = self.peek()?;
                    return Err(ParseError::new(
                        "navigation operator (->, [], .) expected".to_string(),
                        token.line,
                        token.column,
                    ));
                }
                break;
            };
            steps.push(SafeNavigationStep {
                nil_guard,
                operation,
            });
            nil_guard = self.consume_if_symbol(Symbol::Question)?.is_some();
        }
        Ok(steps)
    }

    fn parse_navigation_operation(&mut self) -> Result<Option<NavigationOperation>, ParseError> {
        if let Some(bracket) = self.consume_if_symbol(Symbol::LBracket)? {
            if self.strict_level == 0 {
                return Err(ParseError::new(
                    "unexpected '['".to_string(),
                    bracket.line,
                    bracket.column,
                ));
            }
            if self.check_symbol(Symbol::RBracket)? {
                self.consume()?;
                return Ok(Some(NavigationOperation::ArrayAppend));
            }
            let starts_with_string_token = matches!(&self.peek()?.kind, TokenKind::String(_));
            let index = self.parse_expression()?;
            let index = match (starts_with_string_token, index) {
                (true, Expr::Literal(Literal::String(value))) => {
                    IndexOperand::EmbeddedString(value)
                }
                (_, index) => IndexOperand::Dynamic(Box::new(index)),
            };
            self.expect_symbol(Symbol::RBracket, "expected ']' after index expression")?;
            return Ok(Some(NavigationOperation::Index(index)));
        }
        if let Some(dot) = self.consume_if_symbol(Symbol::Dot)? {
            if self.strict_level < 3 {
                return Err(ParseError::new(
                    "unexpected '.'".to_string(),
                    dot.line,
                    dot.column,
                ));
            }
            let (name, _) = self.expect_identifier("expected property name after '.'")?;
            self.record_synthesized_string_operand(name.clone());
            return Ok(Some(NavigationOperation::Property(name)));
        }
        if self.consume_if_symbol(Symbol::Arrow)?.is_some() {
            let is_optional = self.consume_if_symbol(Symbol::Tilde)?.is_some();
            let (mut name, token) =
                self.expect_identifier_or_c4id("expected property/method name after '->'")?;
            if self.consume_if_symbol(Symbol::ColonColon)?.is_some() {
                let (method_name, _) =
                    self.expect_identifier_or_c4id("expected method name after '::'")?;
                name = format!("{}::{}", name, method_name);
            }
            if self.check_symbol(Symbol::LParen)? {
                self.consume()?;
                let (args, forward_rest) = self.parse_argument_list()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
                return Ok(Some(NavigationOperation::MethodCall {
                    name,
                    args,
                    is_optional,
                    forward_rest,
                }));
            }
            if is_optional {
                return Err(ParseError::new(
                    "'~' requires a method call: expected '(' after method name".to_string(),
                    token.line,
                    token.column,
                ));
            }
            self.record_synthesized_string_operand(name.clone());
            return Ok(Some(NavigationOperation::Property(name)));
        }
        Ok(None)
    }

    fn apply_navigation_operation(expr: Expr, operation: NavigationOperation) -> Expr {
        match operation {
            NavigationOperation::Index(index) => Expr::Index(Box::new(expr), index),
            NavigationOperation::ArrayAppend => Expr::ArrayAppend(Box::new(expr)),
            NavigationOperation::Property(name) => Expr::Property(Box::new(expr), name),
            NavigationOperation::MethodCall {
                name,
                args,
                is_optional,
                forward_rest,
            } => Expr::Call {
                callee: Box::new(Expr::Property(Box::new(expr), name)),
                args,
                is_optional,
                forward_rest,
            },
        }
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
                // Commas in argument lists separate arguments.
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
            // `this` is an ordinary ATT_IDTF in C4Aul. Named bindings win
            // normal variable lookup; only an unresolved name falls back to
            // the context function. Generic postfix parsing also retains the
            // legacy `this(...)` call form and its argument side effects.
            TokenKind::Keyword(Keyword::This) => Ok(Expr::Variable("this".to_string())),
            TokenKind::GlobalCall => {
                let failsafe = self.consume_if_symbol(Symbol::Tilde)?.is_some();
                let (name, _) =
                    self.expect_identifier("expected function name after 'global->'")?;
                self.expect_symbol(Symbol::LParen, "expected '(' after global function name")?;
                let (args, forward_rest) = self.parse_argument_list()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
                Ok(Expr::GlobalCall {
                    name,
                    args,
                    failsafe,
                    forward_rest,
                })
            }
            TokenKind::Identifier(name) => {
                // C4Aul reserves the two unqualified inherited-call names in
                // expression position: the origin script must opt into at
                // least STRICT1 before either form can be parsed
                // (C4AulParse.cpp:2775-2798). Arrow and `global->` calls take
                // separate parser paths and remain ordinary named calls.
                if self.strict_level == 0 && matches!(name.as_str(), "inherited" | "_inherited") {
                    return Err(ParseError::new(
                        "inherited disabled; use #strict syntax!",
                        token.line,
                        token.column,
                    ));
                }
                Ok(Expr::Variable(name))
            }
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
            TokenKind::Symbol(Symbol::LBracket) => {
                if self.strict_level == 0 {
                    return Err(ParseError::new(
                        "unexpected '['".to_string(),
                        token.line,
                        token.column,
                    ));
                }
                self.parse_array_literal()
            }
            TokenKind::Symbol(Symbol::LBrace) => {
                if self.strict_level < 3 {
                    return Err(ParseError::new(
                        "unexpected '{'".to_string(),
                        token.line,
                        token.column,
                    ));
                }
                self.parse_proplist_literal()
            }
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
            // C4Aul emits an ordinary nil value for every empty slot. The
            // initial `]` was handled above so only a non-empty literal can
            // reach the closing-bracket case here (for example, `[1,]`).
            if self.check_symbol(Symbol::Comma)? || self.check_symbol(Symbol::RBracket)? {
                elements.push(Expr::Literal(Literal::Nil));
            } else {
                // Commas in arrays separate elements.
                elements.push(self.parse_assignment()?);
            }
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
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
            // Commas in proplists separate entries.
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

    fn parse_proplist_key(&mut self) -> Result<Expr, ParseError> {
        if self.consume_if_symbol(Symbol::LBracket)?.is_some() {
            let key = self.parse_expression()?;
            self.expect_symbol(Symbol::RBracket, "expected ']' after computed map key")?;
            return Ok(key);
        }

        let token = self.consume()?;
        match token.kind {
            TokenKind::Identifier(name) => {
                self.record_synthesized_string_operand(name.clone());
                Ok(Expr::Literal(Literal::String(name)))
            }
            // C4Aul's parser accepts every ATT_IDTF spelling as a bare map
            // key. Most language words are contextual there; Rust gives
            // those words dedicated tokens, so lower them to the same held
            // string literal as an ordinary identifier. The three literal
            // spellings have distinct native tokens and remain invalid as
            // bare keys (quoted and computed forms are handled above/below).
            TokenKind::Keyword(keyword)
                if !matches!(keyword, Keyword::True | Keyword::False | Keyword::Nil) =>
            {
                let name = keyword.lexeme().to_string();
                self.record_synthesized_string_operand(name.clone());
                Ok(Expr::Literal(Literal::String(name)))
            }
            TokenKind::String(value) => Ok(Expr::Literal(Literal::String(value))),
            _ => Err(ParseError::new(
                "expected identifier, string, or computed key for map key",
                token.line,
                token.column,
            )),
        }
    }

    /// Identifier-backed map/property operands are synthesized by the
    /// parser, unlike quoted strings that the lexer records when first read.
    /// Speculative tokens are replayed without re-lexing, so defer only these
    /// synthesized side effects until the real parse to avoid duplicates.
    fn record_synthesized_string_operand(&mut self, value: String) {
        if self.speculative_tokens.is_none() {
            self.lexer.record_string_operand(value);
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

    fn is_identifier_name_token(kind: &TokenKind) -> bool {
        matches!(kind, TokenKind::Identifier(_))
            || matches!(kind, TokenKind::Keyword(keyword) if *keyword != Keyword::Nil)
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
            // C4Aul declaration words are contextual: the C++ tokenizer
            // emits plain ATT_IDTF for them, so names like `var func, objhgt`
            // (planet/System.c4g/Commits.c:269) are legal. Strict-3 `nil`
            // remains the reserved ATT_NIL token.
            TokenKind::Keyword(keyword) if *keyword != Keyword::Nil => keyword.lexeme().to_string(),
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

    fn expect_identifier_or_c4id(&mut self, message: &str) -> Result<(String, Token), ParseError> {
        let token = self.peek()?.clone();
        let name = match &token.kind {
            TokenKind::Identifier(name) | TokenKind::C4Id(name) => name.clone(),
            TokenKind::Keyword(keyword) if *keyword != Keyword::Nil => keyword.lexeme().to_string(),
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

    fn parse_script_directive(
        &mut self,
        directive: &str,
        directive_line: usize,
        directive_column: usize,
        includes: &mut Vec<String>,
        appends: &mut Vec<AppendTo>,
        strict_level: &mut Option<u8>,
    ) -> Result<(), ParseError> {
        match directive {
            "#include" => {
                let id = self.peek()?.clone();
                let line = id.line;
                let column = id.column;
                match id.kind {
                    TokenKind::C4Id(id) => {
                        self.next()?;
                        let _ = self.peek()?;
                        includes.push(id);
                    }
                    _ => {
                        return Err(ParseError::new(
                            "expected definition ID after #include",
                            line,
                            column,
                        ));
                    }
                }
            }
            "#appendto" => {
                self.lexer.split_next_leading_star();
                let target = self.peek()?.clone();
                let line = target.line;
                let column = target.column;
                match target.kind {
                    TokenKind::C4Id(id) => {
                        self.next()?;
                        // C4Aul accepts the exact lowercase `nowarn` suffix
                        // only after an ID target, never after `*`.
                        let token = self.peek()?.clone();
                        let nowarn =
                            matches!(&token.kind, TokenKind::Identifier(word) if word == "nowarn");
                        if nowarn {
                            self.next()?;
                            let _ = self.peek()?;
                        }
                        appends.push(AppendTo::Id { id, nowarn });
                    }
                    TokenKind::Symbol(Symbol::Star) => {
                        self.next()?;
                        let _ = self.peek()?;
                        appends.push(AppendTo::Wildcard);
                    }
                    _ => {
                        return Err(ParseError::new(
                            "expected definition ID or '*' after #appendto",
                            line,
                            column,
                        ));
                    }
                }
            }
            "#strict" => {
                // C4Aul stores STRICT1 before inspecting the optional level.
                // Bare `#strict` therefore means 1, but an explicit integer
                // is legal only when it is 2 or 3.
                *strict_level = Some(1);
                self.strict_level = 1;
                self.lexer.set_strict_level(1);
                let token = self.peek()?.clone();
                if let TokenKind::Number(level) = token.kind {
                    let raw_level = token.raw_number().unwrap_or(level as u64);
                    if raw_level != 2 && raw_level != 3 {
                        return Err(ParseError::new(
                            "unknown strict level",
                            token.line,
                            token.column,
                        ));
                    }
                    let level = raw_level as u8;
                    *strict_level = Some(level);
                    self.strict_level = level;
                    self.lexer.set_strict_level(level);
                    self.next()?;
                    let _ = self.peek()?;
                }
            }
            _ => {
                return Err(ParseError::new(
                    format!("unknown directive: {directive}"),
                    directive_line,
                    directive_column,
                ));
            }
        }
        Ok(())
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

    /// Parse the preparser-only constant grammar used by `Parse_Const`.
    /// C4Aul consumes exactly one value token here rather than an expression;
    /// the declaration-list parser then requires an immediate comma or
    /// semicolon. This intentionally excludes grouping, calls, containers,
    /// and operators even when the ordinary expression parser could fold
    /// them into a literal-shaped AST.
    fn parse_static_const_initializer(&mut self) -> Result<Expr, ParseError> {
        let token = self.consume()?;
        match token.kind {
            TokenKind::Number(value) => Ok(Expr::Literal(Literal::Int(value))),
            TokenKind::String(value) => {
                // Parse_Const uses Shift(Ref), so this string is owned by the
                // resulting GlobalConsts value rather than the parser Hold
                // ledger maintained for ordinary quoted operands.
                self.lexer.discard_last_string_operand(&value);
                Ok(Expr::Literal(Literal::String(value)))
            }
            TokenKind::C4Id(id) => Ok(Expr::Literal(Literal::C4Id(id))),
            TokenKind::Keyword(Keyword::True) => Ok(Expr::Literal(Literal::Bool(true))),
            TokenKind::Keyword(Keyword::False) => Ok(Expr::Literal(Literal::Bool(false))),
            TokenKind::Keyword(Keyword::Nil) => Ok(Expr::Literal(Literal::Nil)),
            TokenKind::Identifier(name) => Ok(Expr::Variable(name)),
            // C4Aul's declaration words are contextual ATT_IDTF tokens. The
            // Rust lexer distinguishes them for grammar dispatch, but in a
            // constant-value slot they remain named-constant references.
            TokenKind::Keyword(keyword) => Ok(Expr::Variable(keyword.lexeme().to_owned())),
            TokenKind::Symbol(sign @ (Symbol::Plus | Symbol::Minus)) => {
                let number = self.consume()?;
                let number_line = number.line;
                let number_column = number.column;
                let number_is_hex = number.number_is_hex();
                let TokenKind::Number(value) = number.kind else {
                    return Err(ParseError::new(
                        "expected integer after static constant sign",
                        number_line,
                        number_column,
                    ));
                };
                // With Shift(..., false), native C4Aul starts the integer at
                // the sign byte. Consequently `+0x1`/`-0x1` never take the
                // unsigned token's lowercase-hex transition: it returns the
                // signed decimal prefix as zero, then diagnoses the remaining
                // `x...` token when Parse_Const expects a delimiter. Our lexer
                // has already consumed the whole unsigned hex token, so put a
                // synthetic suffix back to preserve both effects.
                if number_is_hex {
                    self.lookahead_buffer.insert(
                        0,
                        Token::new(
                            TokenKind::Identifier("x".to_owned()),
                            number_line,
                            number_column.saturating_add(1),
                        ),
                    );
                    return Ok(Expr::Literal(Literal::Int(0)));
                }
                if sign == Symbol::Minus {
                    Ok(Expr::Unary(
                        UnaryOp::Negate,
                        Box::new(Expr::Literal(Literal::Int(value))),
                    ))
                } else {
                    Ok(Expr::Literal(Literal::Int(value)))
                }
            }
            _ => Err(ParseError::new(
                "expected static constant value",
                token.line,
                token.column,
            )),
        }
    }

    fn parse_var_decl_list(&mut self, kind: VarDeclKind) -> Result<(), ParseError> {
        let mut starts_declaration_group = true;
        // Parse a comma-separated name list. Static constants additionally
        // require an initializer for every name.
        loop {
            // Parse variable name
            let (name, name_token) =
                self.expect_identifier("expected variable name in declaration")?;

            let init = if kind == VarDeclKind::StaticConst {
                // Parse exactly one preparser constant token; the declaration
                // list owns and validates the following comma/semicolon.
                if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                    Some(self.parse_static_const_initializer()?)
                } else {
                    return Err(ParseError::new(
                        "static const declaration requires an initializer",
                        name_token.line,
                        name_token.column,
                    ));
                }
            } else {
                // The C++ preparser registers each name before validating its
                // delimiter, so even `local value = 5;` retains `value` in
                // declaration metadata before reporting the syntax error.
                self.script_var_decls.push(VarDecl {
                    kind,
                    name: name.clone(),
                    init: None,
                    starts_declaration_group,
                });
                if let Some(equal) = self.consume_if_symbol(Symbol::Equal)? {
                    return Err(ParseError::new(
                        "expected ',' or ';' after variable declaration",
                        equal.line,
                        equal.column,
                    ));
                }
                None
            };

            if kind == VarDeclKind::StaticConst {
                self.script_var_decls.push(VarDecl {
                    kind,
                    name,
                    init,
                    starts_declaration_group,
                });
            }
            starts_declaration_group = false;

            // Check for comma (more declarations) or semicolon (end)
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue; // Parse next declaration
            } else {
                break;
            }
        }

        // Expect semicolon
        self.expect_symbol(Symbol::Semicolon, "expected ';' after variable declaration")?;

        Ok(())
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

    fn parse_expression_at_strict(source: &str, strict_level: u8) -> Result<Expr, ParseError> {
        Parser::with_strict_level(source, Some(strict_level)).parse_direct_exec_expression()
    }

    #[test]
    fn directives_require_four_byte_c4id_targets() {
        let valid = r#"
#include CLNK
#include 1HUD
#include AB_1
#appendto CLNK nowarn
#appendto 1HUD
#appendto AB_1
#appendto*
func Ok() { return 1; }
"#;
        let direct = parse_script(valid).expect("C4ID and wildcard targets parse");
        assert_eq!(direct.includes, ["CLNK", "1HUD", "AB_1"]);
        assert_eq!(
            direct.appends,
            [
                AppendTo::Id {
                    id: "CLNK".into(),
                    nowarn: true,
                },
                AppendTo::Id {
                    id: "1HUD".into(),
                    nowarn: false,
                },
                AppendTo::Id {
                    id: "AB_1".into(),
                    nowarn: false,
                },
                AppendTo::Wildcard,
            ]
        );
        let recovered = crate::Script::compile(valid).expect("recovering parser loads script");
        assert!(recovered.parse_diagnostics().is_empty());
        assert_eq!(recovered.includes(), ["CLNK", "1HUD", "AB_1"]);
        assert_eq!(recovered.appends(), direct.appends);

        for directive in ["#include", "#appendto"] {
            for target in ["clnk", "ABC", "ABCDE", "Definition", "1234"] {
                let source = format!("{directive} {target}\nfunc Ok() {{ return 1; }}");
                let error = parse_script(&source).expect_err("non-C4ID target must be rejected");
                assert!(
                    error.message().contains("expected definition ID"),
                    "unexpected error for {source:?}: {error}"
                );

                let recovered =
                    crate::Script::compile(&source).expect("directive error is recoverable");
                assert!(
                    recovered
                        .parse_diagnostics()
                        .iter()
                        .any(|error| error.message().contains("expected definition ID")),
                    "missing target diagnostic for {source:?}"
                );
                assert!(recovered.includes().is_empty(), "source: {source}");
                assert!(recovered.appends().is_empty(), "source: {source}");
                assert!(recovered.functions().contains_key("Ok"), "source: {source}");
            }
        }

        let wildcard_nowarn = "#appendto * nowarn";
        assert!(parse_script(wildcard_nowarn).is_err());
        let recovered =
            crate::Script::compile(wildcard_nowarn).expect("stray nowarn is recoverable");
        assert_eq!(recovered.appends(), [AppendTo::Wildcard]);
        assert!(
            !recovered.parse_diagnostics().is_empty(),
            "nowarn is legal only after a C4ID target"
        );

        for source in ["#appendto**", "#appendto*=", "#appendto**="] {
            assert!(parse_script(source).is_err(), "source: {source}");
            let recovered = crate::Script::compile(source).expect("operator tail recovers");
            assert_eq!(
                recovered.appends(),
                [AppendTo::Wildcard],
                "source: {source}"
            );
            assert!(
                !recovered.parse_diagnostics().is_empty(),
                "source: {source}"
            );
        }

        let wrong_case = "#Include CLNK";
        let error = parse_script(wrong_case).expect_err("directives are case-sensitive");
        assert!(error.message().contains("unknown directive"));
        let recovered = crate::Script::compile(wrong_case).expect("unknown directive recovers");
        assert!(recovered
            .parse_diagnostics()
            .iter()
            .any(|error| error.message().contains("unknown directive")));

        let recovered = crate::Script::compile("#include #strict 2\nfunc Ok() { return 1; }")
            .expect("invalid include target recovers at that target");
        assert_eq!(recovered.strict_level(), Some(2));
        assert!(recovered.functions().contains_key("Ok"));
        assert!(recovered.includes().is_empty());

        let recovered = crate::Script::compile("#appendto func Ok() { return 1; }")
            .expect("invalid append target recovers at that target");
        assert!(recovered.functions().contains_key("Ok"));
        assert!(recovered.appends().is_empty());

        for source in ["#include 1HUD(", "#appendto 1HUD:"] {
            let error = parse_script(source).expect_err("call/label token is not a C4ID");
            assert!(error.message().contains("expected definition ID"));
            let recovered = crate::Script::compile(source).expect("target error recovers");
            assert!(recovered.includes().is_empty(), "source: {source}");
            assert!(recovered.appends().is_empty(), "source: {source}");
        }

        for source in [
            "#include CLNK '",
            "#appendto CLNK '",
            "#appendto CLNK nowarn '",
            "#appendto * '",
        ] {
            let recovered =
                crate::Script::compile(source).expect("post-directive lexer error recovers");
            assert!(
                !recovered.parse_diagnostics().is_empty(),
                "source: {source}"
            );
            assert!(recovered.includes().is_empty(), "source: {source}");
            assert!(recovered.appends().is_empty(), "source: {source}");
        }

        for source in [
            "#strict 2\n#include ABCD\nCLNK(",
            "#strict 2\n#appendto ABCD\nCLNK(",
            "#strict 2\n#appendto *\nCLNK(",
        ] {
            let recovered = crate::Script::compile(source).expect("lookahead error recovers");
            assert!(recovered.includes().is_empty(), "source: {source}");
            assert!(recovered.appends().is_empty(), "source: {source}");
            assert!(
                recovered
                    .parse_diagnostics()
                    .iter()
                    .any(|error| error.message().contains("stupid func label: CLNK")),
                "source: {source}; diagnostics: {:?}",
                recovered.parse_diagnostics()
            );
        }

        for prefix in ["", "#strict\n"] {
            let source = format!("{prefix}#include ABCD\nCLNK(");
            let recovered = crate::Script::compile(&source).expect("legacy warning recovers");
            assert_eq!(recovered.includes(), ["ABCD"], "source: {source}");
            assert!(
                recovered
                    .parse_diagnostics()
                    .iter()
                    .any(|error| error.message().contains("stupid func label: CLNK")),
                "source: {source}; diagnostics: {:?}",
                recovered.parse_diagnostics()
            );
        }
    }

    #[test]
    fn explicit_strict_one_is_rejected_but_bare_strict_is_one() {
        let bare = "#strict\nfunc Ok() { return 1; }";
        assert_eq!(
            parse_script(bare).expect("bare strict parses").strict_level,
            Some(1)
        );
        let recovered = crate::Script::compile(bare).expect("bare strict compiles");
        assert_eq!(recovered.strict_level(), Some(1));
        assert!(recovered.parse_diagnostics().is_empty());

        for source in ["#strict 2(", "#strict 0x2("] {
            assert!(parse_script(source).is_err(), "source: {source}");
            let recovered = crate::Script::compile(source).expect("adjacency error recovers");
            assert_eq!(recovered.strict_level(), Some(1), "source: {source}");
            assert!(
                !recovered.parse_diagnostics().is_empty(),
                "source: {source}"
            );
        }

        for (source, function_name) in [("#strict 2:", "2"), ("#strict 0x2:", "0x2")] {
            let direct = parse_script(source).expect("colon begins a legacy function");
            assert_eq!(direct.strict_level, Some(1), "source: {source}");
            assert_eq!(direct.functions[0].name, function_name, "source: {source}");
            let recovered = crate::Script::compile(source).expect("legacy function compiles");
            assert_eq!(recovered.strict_level(), Some(1), "source: {source}");
            assert!(recovered.functions().contains_key(function_name));
            assert!(
                !recovered.parse_diagnostics().is_empty(),
                "source: {source}"
            );
        }

        for (spelling, expected) in [("2", 2), ("3", 3), ("02", 2), ("0x2", 2)] {
            let source = format!("#strict /* separator */ {spelling}\nfunc Ok() {{ return 1; }}");
            assert_eq!(
                parse_script(&source)
                    .unwrap_or_else(|error| panic!("{source:?} should parse: {error}"))
                    .strict_level,
                Some(expected)
            );
            let recovered = crate::Script::compile(&source).expect("valid strict compiles");
            assert_eq!(recovered.strict_level(), Some(expected));
            assert!(recovered.parse_diagnostics().is_empty(), "source: {source}");
        }

        for spelling in ["0", "1", "4", "4294967298", "0x100000002"] {
            let source = format!("#strict {spelling}\nfunc Ok() {{ return 1; }}");
            let error = parse_script(&source).expect_err("invalid explicit strict must fail");
            assert_eq!(error.message(), "unknown strict level");

            let recovered =
                crate::Script::compile(&source).expect("strict-level error is recoverable");
            assert_eq!(recovered.strict_level(), Some(1));
            assert!(recovered.functions().contains_key("Ok"), "source: {source}");
            assert!(
                recovered
                    .parse_diagnostics()
                    .iter()
                    .any(|error| error.message() == "unknown strict level"),
                "missing strict-level diagnostic for {source:?}"
            );
        }
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

    // C4Aul binds `!` before `=`: the assignment targets the resulting
    // boolean value and fails its reference conversion at runtime.
    #[test]
    fn bang_assignment_targets_the_negated_value() {
        let script = parse_script(r#"func Test() { var x; return !x = 42; }"#).expect("parses");
        let function = &script.functions[0];
        let Stmt::Return(Some(expr)) = &function.body[1] else {
            panic!("expected return with expression");
        };
        assert!(
            matches!(
                expr,
                Expr::Assignment(
                    AssignmentTarget::InvalidValue {
                        expression: left,
                        operator: "="
                    },
                    value
                )
                    if matches!(&**left, Expr::Unary(UnaryOp::Not, operand)
                        if matches!(&**operand, Expr::Variable(name) if name == "x"))
                        && matches!(&**value, Expr::Literal(Literal::Int(42)))
            ),
            "expected (!x) = 42 shape, got {expr:?}"
        );
    }

    #[test]
    fn parse_return_with_simple_expression() {
        let result = parse_script("func Test() { return 42; }");
        assert!(result.is_ok());
    }

    #[test]
    fn strict_two_rejects_textual_string_comparison_operators() {
        for operator in ["eq", "ne"] {
            let source = format!("#strict 2\nfunc Test() {{ return \"a\" {operator} \"a\"; }}");
            assert!(
                parse_script(&source).is_err(),
                "{operator} must be an identifier rather than an operator at strict two"
            );
        }
    }

    #[test]
    fn nil_is_contextual_below_strict_three() {
        for strict_level in 0..3 {
            let expression = parse_expression_at_strict("nil", strict_level)
                .expect("nil remains an identifier below strict three");
            assert!(
                matches!(expression, Expr::Variable(name) if name == "nil"),
                "strict level {strict_level} must bind nil as an identifier"
            );

            let mut parser =
                Parser::with_strict_level("func Echo(nil) { return nil; }", Some(strict_level));
            let script = parser
                .parse_script()
                .expect("nil is legal as a bound parameter below strict three");
            assert_eq!(script.functions[0].params[0].name, "nil");
            assert!(matches!(
                &script.functions[0].body[0],
                Stmt::Return(Some(Expr::Variable(name))) if name == "nil"
            ));
        }

        assert!(matches!(
            parse_expression_at_strict("nil", 3).expect("strict-three nil literal parses"),
            Expr::Literal(Literal::Nil)
        ));
        assert!(matches!(
            parse_expression_at_strict("Nil", 3).expect("reserved nil is case-sensitive"),
            Expr::Variable(name) if name == "Nil"
        ));

        for source in ["func nil() {}", "func Echo(nil) { return nil; }"] {
            let mut parser = Parser::with_strict_level(source, Some(3));
            assert!(
                parser.parse_script().is_err(),
                "strict-three nil must stay reserved in {source:?}"
            );
        }
    }

    #[test]
    fn array_syntax_requires_strict_one() {
        for source in ["[1]", "value[0]", "value[\"key\"]", "value[]"] {
            let error = parse_expression_at_strict(source, 0)
                .expect_err("NONSTRICT array syntax must be rejected");
            assert_eq!(error.message(), "unexpected '['", "source: {source}");

            for strict_level in 1..=3 {
                parse_expression_at_strict(source, strict_level).unwrap_or_else(|error| {
                    panic!("strict level {strict_level} must accept {source:?}: {error}")
                });
            }
        }
    }

    #[test]
    fn map_and_dot_require_strict_three() {
        for strict_level in 0..3 {
            let map_error = parse_expression_at_strict("{key = 1}", strict_level)
                .expect_err("pre-strict-three map literals must be rejected");
            assert_eq!(map_error.message(), "unexpected '{'");

            let dot_error = parse_expression_at_strict("value.key", strict_level)
                .expect_err("pre-strict-three dot access must be rejected");
            assert_eq!(dot_error.message(), "unexpected '.'");

            parse_expression_at_strict("1 .. 2", strict_level)
                .expect("concatenation dots are not property access");
            parse_expression_at_strict("value->Get()", strict_level)
                .expect("arrow navigation is not dot access");
        }

        parse_expression_at_strict("{key = 1}", 3).expect("strict-three map literal parses");
        parse_expression_at_strict("value.key", 3).expect("strict-three dot access parses");

        let reserved_key = parse_expression_at_strict("value.nil", 3)
            .expect_err("strict-three nil is not an identifier after dot");
        assert_eq!(reserved_key.message(), "expected property name after '.'");
    }

    #[test]
    fn invented_word_operators_are_ordinary_identifiers() {
        for expression in [
            "1 lt 2",
            "1 le 2",
            "1 gt 2",
            "1 ge 2",
            "true and false",
            "true or false",
            "not false",
        ] {
            let source = format!("func Test() {{ if ({expression}) return 1; }}");
            assert!(
                parse_script(&source).is_err(),
                "invented operator expression {expression:?} must fail"
            );
        }

        parse_script(
            "#strict\nfunc Test() { \
                 var lt, le, gt, ge, and, or, not; \
                 lt = 1; le = 2; gt = 3; ge = 4; and = 5; or = 6; not = 7; \
                 return [lt, le, gt, ge, and, or, not]; \
             }",
        )
        .expect("operator-like words remain valid identifier names");
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
        let script = parse_script("#strict\nfunc Test() { var marker; [marker]; return 42; }")
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
        let result = parse_script("#strict\nfunc Test() { var arr = [1, 2]; arr[0] = 3; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_nested_array_index_assignment() {
        let result = parse_script("#strict\nfunc Test() { var m = [[1]]; m[0][0] = 2; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_proplist_property_assignment() {
        let result = parse_script("#strict 3\nfunc Test() { var obj = {}; obj.prop = 1; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_nested_proplist_assignment() {
        let result = parse_script("#strict 3\nfunc Test() { var obj = {n={}}; obj.n.prop = 1; }");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_map_literal_computed_keys_as_expressions() {
        let script = parse_script(
            r#"#strict 3
            func Test(object_key) {
                return { bare = 1, "quoted" = 2, [42] = 3, [CLNK] = 4, [object_key] = 5 };
            }"#,
        )
        .expect("computed map keys parse");
        let Stmt::Return(Some(Expr::Proplist(entries))) = &script.functions[0].body[0] else {
            panic!("expected returned map literal");
        };

        assert_eq!(entries.len(), 5);
        assert!(matches!(
            &entries[0].0,
            Expr::Literal(Literal::String(key)) if key == "bare"
        ));
        assert!(matches!(
            &entries[1].0,
            Expr::Literal(Literal::String(key)) if key == "quoted"
        ));
        assert!(matches!(&entries[2].0, Expr::Literal(Literal::Int(42))));
        assert!(matches!(
            &entries[3].0,
            Expr::Literal(Literal::C4Id(id)) if id == "CLNK"
        ));
        assert!(matches!(
            &entries[4].0,
            Expr::Variable(name) if name == "object_key"
        ));
    }

    #[test]
    fn contextual_keyword_map_keys_match_att_idtf() {
        // C++ tokenizes all of these spellings as ATT_IDTF. The first group
        // exercises every contextual word that the Rust lexer currently
        // promotes to Keyword; the rest pin already-identifier spellings and
        // case-sensitive literal lookalikes at the same map-key boundary.
        let expected_keys = [
            "func",
            "global",
            "private",
            "protected",
            "public",
            "local",
            "var",
            "static",
            "const",
            "if",
            "else",
            "while",
            "for",
            "in",
            "return",
            "break",
            "continue",
            "this",
            "int",
            "bool",
            "string",
            "object",
            "id",
            "array",
            "proplist",
            "effect",
            "eq",
            "ne",
            "lt",
            "le",
            "gt",
            "ge",
            "and",
            "or",
            "not",
            "True",
            "False",
            "Nil",
            "NIL",
            "Global",
            "GLOBAL",
        ];
        let entries = expected_keys
            .iter()
            .enumerate()
            .map(|(value, key)| format!("{key} = {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        let script = parse_script(&format!(
            "#strict 3\nfunc Test() {{ return {{ {entries} }}; }}"
        ))
        .expect("native ATT_IDTF spellings parse as bare map keys");
        let Stmt::Return(Some(Expr::Proplist(parsed_entries))) = &script.functions[0].body[0]
        else {
            panic!("expected returned map literal");
        };
        let parsed_keys = parsed_entries
            .iter()
            .map(|(key, _)| match key {
                Expr::Literal(Literal::String(key)) => key.as_str(),
                other => panic!("expected string map key, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(parsed_keys, expected_keys);
        assert_eq!(
            script.string_literals, expected_keys,
            "contextual bare keys are held in native encounter order"
        );

        let preserved = parse_script(
            r#"#strict 3
               func Test() {
                   return { "true" = 1, "false" = 2, "nil" = 3,
                            [true] = 4, [false] = 5, [nil] = 6 };
               }"#,
        )
        .expect("reserved literal spellings remain valid when quoted or computed");
        assert!(matches!(
            &preserved.functions[0].body[0],
            Stmt::Return(Some(Expr::Proplist(entries))) if entries.len() == 6
        ));
        assert_eq!(preserved.string_literals, ["true", "false", "nil"]);

        for key in ["true", "false", "nil"] {
            let error = parse_script(&format!(
                "#strict 3\nfunc Test() {{ return {{ {key} = 1 }}; }}"
            ))
            .expect_err("native boolean/nil tokens are not bare map keys");
            assert_eq!(
                error.message(),
                "expected identifier, string, or computed key for map key"
            );
        }

        for key in ["global->Call()", "CLNK", "TRUE"] {
            let error = parse_script(&format!(
                "#strict 3\nfunc Test() {{ return {{ {key} = 1 }}; }}"
            ))
            .expect_err("non-ATT_IDTF tokens are not bare map keys");
            assert_eq!(
                error.message(),
                "expected identifier, string, or computed key for map key"
            );
        }
    }

    #[test]
    fn speculative_unary_records_synthesized_string_operands_once() {
        let script = parse_script(
            r#"#strict 3
               func Test(target) {
                   return [!{if = 1}, !{"quoted" = 2}, !target.dot, !target->arrow];
               }"#,
        )
        .expect("unary operands parse after their speculative preflight");

        assert_eq!(
            script.string_literals,
            ["if", "quoted", "dot", "arrow"],
            "replayed identifier-backed operands are held exactly once"
        );
    }

    #[test]
    fn link_string_operands_include_map_and_property_identifier_keys_in_order() {
        let script = parse_script(
            r#"#strict 3
               func Test(target) {
                   var map = {
                       bare = "value",
                       "quoted" = 2,
                       ["computed"] = 3
                   };
                   return [target.dot, target->arrow, target->Method()];
               }"#,
        )
        .expect("map and property string operands parse");

        assert_eq!(
            script.string_literals,
            ["bare", "value", "quoted", "computed", "dot", "arrow"],
            "C4Aul links map/property keys, but not arrow method names, in encounter order"
        );
    }

    #[test]
    fn static_const_string_is_not_a_held_link_literal() {
        let script = parse_script(
            r#"#strict 3
               static const LABEL = "constant";
               func Test() { return ["ordinary", { key = 1 }]; }"#,
        )
        .expect("string constant and ordinary literals parse");

        assert!(matches!(
            script.var_decls.as_slice(),
            [VarDecl {
                kind: VarDeclKind::StaticConst,
                init: Some(Expr::Literal(Literal::String(value))),
                ..
            }] if value == "constant"
        ));
        assert_eq!(script.string_literals, ["ordinary", "key"]);
    }

    #[test]
    fn statement_leading_map_literal_is_not_a_block() {
        let script = parse_script(
            r#"#strict 3
            func Test(key) {
                { [key] = SideEffect(), nested = { value = 1 } };
                { key = SideEffect(); }
                { if (key = 1) {} }
                {}
            }"#,
        )
        .expect("map-expression statements and ordinary blocks disambiguate");

        let body = &script.functions[0].body;
        assert!(matches!(
            &body[0],
            Stmt::Expr(Expr::Proplist(entries)) if entries.len() == 2
        ));
        assert!(matches!(
            &body[1],
            Stmt::Block(statements)
                if matches!(statements.as_slice(), [Stmt::Assignment { .. }])
        ));
        assert!(matches!(
            &body[2],
            Stmt::Block(statements) if matches!(statements.as_slice(), [Stmt::If { .. }])
        ));
        assert!(matches!(&body[3], Stmt::Block(statements) if statements.is_empty()));

        let error = parse_script("#strict 2\nfunc Test() { { key = 1 }; }")
            .expect_err("statement-leading maps remain STRICT3-only");
        assert_eq!(error.message(), "unexpected '{'");
    }

    #[test]
    fn statement_map_probe_discards_lookahead_string_operands() {
        let mut parser = Parser::new("#strict 3\nfunc Broken() { { var = \"\\q\"; } }");
        let error = parser
            .parse_script()
            .expect_err("the malformed declaration must stop before its string operand");

        assert_eq!(error.message(), "expected variable name");
        assert!(
            parser.lexer.take_string_literals().is_empty(),
            "C++ IsMapLiteral uses Discard; lookahead-only strings are not linked"
        );
        let diagnostics = parser.lexer.take_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message(), "unknown escape: q");
    }

    #[test]
    fn statement_map_probe_lexer_error_keeps_cpp_cursor_progress() {
        let mut parser = Parser::new("#strict 3\nfunc Broken() { { key = \"discarded\" @ }; }");
        let error = parser
            .parse_script()
            .expect_err("invalid lookahead character must abort the statement probe");

        assert_eq!(error.message(), "unexpected character '@'");
        assert!(parser.lexer.take_string_literals().is_empty());
        assert!(matches!(
            parser
                .peek()
                .expect("cursor remains after the bad byte")
                .kind,
            TokenKind::Symbol(Symbol::RBrace)
        ));
    }

    #[test]
    fn computed_map_key_requires_closing_bracket() {
        let error = parse_script("#strict 3\nfunc Test() { return { [42) = 1 }; }")
            .expect_err("unterminated computed key must fail");
        assert!(error
            .message()
            .contains("expected ']' after computed map key"));
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

    #[test]
    fn map_foreach_headers_capture_both_binders() {
        for (header, expected_declare) in [
            ("var key, value in map", true),
            ("key, value in map", false),
        ] {
            let script = parse_script(&format!(
                "func Test() {{ var key, value, map; for ({header}) {{ break; }} }}"
            ))
            .expect("map foreach parses");
            let foreach = script.functions[0]
                .body
                .iter()
                .find(|statement| matches!(statement, Stmt::ForIn { .. }))
                .expect("foreach statement retained in AST");

            assert!(matches!(
                foreach,
                Stmt::ForIn {
                    variable,
                    value_variable: Some(value_variable),
                    declare_var,
                    body,
                    ..
                } if variable == "key"
                    && value_variable == "value"
                    && *declare_var == expected_declare
                    && matches!(body.as_slice(), [Stmt::Break])
            ));
        }
    }

    #[test]
    fn map_foreach_accepts_contextual_keywords_as_binders() {
        let script =
            parse_script("func Test() { var func, while, map; for (var func, while in map) {} }")
                .expect("contextual-keyword map binders parse");

        assert!(matches!(
            &script.functions[0].body[1],
            Stmt::ForIn {
                variable,
                value_variable: Some(value_variable),
                ..
            } if variable == "func" && value_variable == "while"
        ));
    }

    #[test]
    fn foreach_probe_keeps_declaration_commas_but_not_comma_expressions() {
        let script = parse_script("func Test() { for (var i = 0, j = 1; i < 3; ++i) {} }")
            .expect("C-style declaration loop parses");

        assert!(matches!(
            &script.functions[0].body[0],
            Stmt::For {
                init: Some(crate::ast::ForInit::VarDecls(declarations)),
                ..
            } if declarations.len() == 2
        ));
        assert!(parse_script("func Test() { for (i = 0, j = 1; i < 3; ++i) {} }").is_err());
    }

    #[test]
    fn only_raw_string_indexes_use_the_embedded_operand_ast() {
        let embedded =
            parse_expression_at_strict(r#"map["key"]"#, 3).expect("raw string index parses");
        assert!(matches!(
            embedded,
            Expr::Index(_, IndexOperand::EmbeddedString(key)) if key == "key"
        ));

        let dynamic = parse_expression_at_strict(r#"map[("key")]"#, 3)
            .expect("parenthesized string index parses");
        assert!(matches!(
            dynamic,
            Expr::Index(
                _,
                IndexOperand::Dynamic(index)
            ) if matches!(
                index.as_ref(),
                Expr::Literal(Literal::String(key)) if key == "key"
            )
        ));

        let assignment = parse_script(
            r#"#strict 3
            func Test() { map["key"] = 1; map[("key")] = 2; }
            "#,
        )
        .expect("string-index assignments parse");
        assert!(matches!(
            &assignment.functions[0].body[..],
            [
                Stmt::Assignment {
                    target: AssignmentTarget::Index(
                        _,
                        IndexOperand::EmbeddedString(embedded)
                    ),
                    ..
                },
                Stmt::Assignment {
                    target: AssignmentTarget::Index(_, IndexOperand::Dynamic(_)),
                    ..
                }
            ] if embedded == "key"
        ));

        let navigation = parse_expression_at_strict(r#"map?["key"]?[(("key"))]"#, 3)
            .expect("safe string-index navigation parses");
        assert!(matches!(
            navigation,
            Expr::SafeNavigation { steps, .. }
                if matches!(
                    &steps[..],
                    [
                        SafeNavigationStep {
                            operation: NavigationOperation::Index(
                                IndexOperand::EmbeddedString(embedded)
                            ),
                            ..
                        },
                        SafeNavigationStep {
                            operation: NavigationOperation::Index(
                                IndexOperand::Dynamic(_)
                            ),
                            ..
                        }
                    ] if embedded == "key"
                )
        ));
    }
}
