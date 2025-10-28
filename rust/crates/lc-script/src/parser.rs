use crate::ast::{AccessLevel, AppendTo, AssignmentTarget, BinaryOp, Expr, Function, Parameter, Script, Stmt, TypeAnnotation, UnaryOp, VarDecl, VarDeclKind};
use crate::error::ParseError;
use crate::lexer::Lexer;
use crate::token::{Keyword, Symbol, Token, TokenKind};
use crate::value::Literal;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    // Additional buffer for multi-token lookahead (used in for-loop disambiguation)
    lookahead_buffer: Vec<Token>,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            lexer: Lexer::new(source),
            peeked: None,
            lookahead_buffer: Vec::new(),
        }
    }

    pub fn parse_script(&mut self) -> Result<Script, ParseError> {
        // Parse directives, variable declarations, and functions
        // Directives and variable declarations can be interspersed
        let mut includes = Vec::new();
        let mut appendto = None;
        let mut strict_level = None;
        let mut var_decls = Vec::new();
        let mut functions = Vec::new();

        while !self.is_eof()? {
            // Check for directives
            if let Some(directive) = self.try_parse_directive()? {
                match directive.as_str() {
                    "#include" => {
                        let id = self.expect_identifier("expected definition ID after #include")?;
                        if let TokenKind::Identifier(id_str) = id.kind {
                            includes.push(id_str);
                        }
                    }
                    "#appendto" => {
                        let next = self.next()?;
                        appendto = Some(match &next.kind {
                            TokenKind::Identifier(id) => AppendTo::Id(id.clone()),
                            TokenKind::Symbol(Symbol::Star) => AppendTo::Wildcard,
                            _ => {
                                return Err(ParseError::new(
                                    "expected definition ID or '*' after #appendto",
                                    next.line,
                                    next.column,
                                ))
                            }
                        });
                    }
                    "#strict" => {
                        // Default to level 1
                        let mut level = 1;
                        // Check if there's a number following
                        if let Ok(token) = self.peek() {
                            if let TokenKind::Number(n) = token.kind {
                                if n >= 1 && n <= 3 {
                                    level = n as u8;
                                    self.next()?; // consume the number
                                }
                            }
                        }
                        strict_level = Some(level);
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

        Ok(Script::with_directives(functions, var_decls, includes, appendto, strict_level))
    }

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

        self.expect_keyword(Keyword::Func, "expected 'func' declaration")?;
        let name_token = self.expect_identifier("expected function name")?;
        let name = if let TokenKind::Identifier(name) = name_token.kind.clone() {
            name
        } else {
            unreachable!()
        };
        self.expect_symbol(Symbol::LParen, "expected '(' after function name")?;
        let params = self.parse_parameter_list()?;
        self.expect_symbol(Symbol::RParen, "expected ')' after parameter list")?;
        self.expect_symbol(Symbol::LBrace, "expected '{' to start function body")?;
        let body = self.parse_block_statements()?;
        self.expect_symbol(Symbol::RBrace, "expected '}' after function body")?;

        Ok(Function {
            name,
            params,
            body,
            access,
        })
    }

    fn parse_parameter_list(&mut self) -> Result<Vec<Parameter>, ParseError> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen)? {
            return Ok(params);
        }
        loop {
            // Check for optional type annotation
            let type_annotation = self.parse_type_annotation()?;

            // Check for optional reference parameter (&)
            let is_reference = self.consume_if_symbol(Symbol::Ampersand)?.is_some();

            let token = self.expect_identifier("expected parameter name")?;
            if let TokenKind::Identifier(name) = token.kind {
                if is_reference || type_annotation.is_some() {
                    params.push(Parameter::with_reference(name, type_annotation, is_reference));
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
            TokenKind::Identifier(name) => {
                match name.as_str() {
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
                    _ => return Ok(None),
                }
            }
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
                    TokenKind::Identifier(name) => {
                        match name.as_str() {
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
                            _ => {
                                return Err(ParseError::new(
                                    "expected type name after '|' in union type".to_string(),
                                    next_token.line,
                                    next_token.column,
                                ))
                            }
                        }
                    }
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
        if let Some(return_token) = self.consume_if_keyword(Keyword::Return)? {
            return self.parse_return(return_token);
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

        // Check for context annotation vs array literal
        // Context annotations: [$LocaleKey$|...] or [Key=Value|...] or [Key|...]
        // Arrays: [expr, expr, ...]
        if self.check_symbol(Symbol::LBracket)? {
            // Consume the '['
            self.consume()?;
            // Use lookahead to distinguish context annotation from array
            if self.is_context_annotation()? {
                // This is a context annotation, parse it
                return self.parse_context_annotation_body();
            } else {
                // This is an array literal, we need to handle it as an expression
                // But we already consumed the '[', so we need to parse it differently
                // For now, return an error - we'll fix array literal parsing separately
                let next_token = self.peek()?;
                return Err(ParseError::new(
                    "array literals in statement position not yet supported",
                    next_token.line,
                    next_token.column,
                ));
            }
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
            let name_token = self.expect_identifier("expected variable name")?;
            let name = if let TokenKind::Identifier(name) = name_token.kind {
                name
            } else {
                unreachable!()
            };
            let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                Some(self.parse_expression()?)
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

        // Return single declaration if only one, otherwise return block
        if decls.len() == 1 {
            Ok(decls.into_iter().next().unwrap())
        } else {
            Ok(Stmt::Block(decls))
        }
    }

    fn parse_return(&mut self, return_token: Token) -> Result<Stmt, ParseError> {
        // return_token is the consumed Return keyword token

        // Handle: return;
        if self.consume_if_symbol(Symbol::Semicolon)?.is_some() {
            return Ok(Stmt::Return(None));
        }

        // Check if lparen immediately follows return (no space)
        // Use column numbers to detect: if return ends at col X and ( starts at col X+6 ("return".len())
        // then they're adjacent (no space)
        let lparen_immediately_follows = if self.check_symbol(Symbol::LParen)? {
            let lparen_token = self.peek()?;
            // "return" is 6 characters, so if lparen is at return_col + 6, they're adjacent
            return_token.column + 6 == lparen_token.column
        } else {
            false
        };

        // Only intercept return(...) forms with NO space between return and (
        if lparen_immediately_follows {
            let lparen = self.consume()?; // consume '('

            // Handle: return(); (empty parentheses)
            if self.check_symbol(Symbol::RParen)? {
                self.consume()?; // consume ')'
                self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;
                return Ok(Stmt::Return(None));
            }

            // Parse first expression to check for comma operator
            let first_expr = self.parse_expression()?;

            // Check if this is comma-separated expressions: return(expr1, expr2, ...);
            if self.check_symbol(Symbol::Comma)? {
                // Definitely comma syntax - parse all comma-separated expressions
                let mut exprs = vec![first_expr];

                while self.consume_if_symbol(Symbol::Comma)?.is_some() {
                    exprs.push(self.parse_expression()?);
                }

                self.expect_symbol(Symbol::RParen, "expected ')' after expression")?;
                self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;

                // Multiple expressions: desugar to block with expr statements + return
                // This preserves side effects of all expressions and returns the last one
                let mut stmts = Vec::new();
                let last_expr = exprs.pop().unwrap();
                for expr in exprs {
                    stmts.push(Stmt::Expr(expr));
                }
                stmts.push(Stmt::Return(Some(last_expr)));
                return Ok(Stmt::Block(stmts));
            }

            // No comma - it's return(single_expr);
            // Consume ) and ; and return the single expression
            self.expect_symbol(Symbol::RParen, "expected ')' after expression")?;
            self.expect_symbol(Symbol::Semicolon, "expected ';' after return statement")?;
            return Ok(Stmt::Return(Some(first_expr)));
        }

        // Handle: return expr; (normal expression parsing, includes "return (expr) op expr;")
        let expr = self.parse_expression()?;
        self.expect_symbol(Symbol::Semicolon, "expected ';' after return value")?;
        Ok(Stmt::Return(Some(expr)))
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
            let name_token = self.expect_identifier("expected variable name")?;
            let variable = if let TokenKind::Identifier(name) = name_token.kind {
                name
            } else {
                unreachable!()
            };

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
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                decls.push((variable, first_init));

                // Parse additional comma-separated variables
                while self.consume_if_symbol(Symbol::Comma)?.is_some() {
                    let name_token = self.expect_identifier("expected variable name")?;
                    let name = if let TokenKind::Identifier(name) = name_token.kind {
                        name
                    } else {
                        unreachable!()
                    };
                    let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                        Some(self.parse_expression()?)
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
        &self,
        expr: Expr,
        eq_token: &Token,
    ) -> Result<AssignmentTarget, ParseError> {
        match expr {
            Expr::Variable(name) => Ok(AssignmentTarget::Variable(name)),
            Expr::Property(base, name) => {
                let base_target = self.expression_to_assignment_target(*base, eq_token)?;
                Ok(AssignmentTarget::Property(Box::new(base_target), name))
            }
            // Array/proplist indexing is assignable: arr[i] = value
            Expr::Index(base, index) => {
                let base_target = self.expression_to_assignment_target(*base, eq_token)?;
                Ok(AssignmentTarget::Index(Box::new(base_target), index))
            }
            // Special case: Local(expr), Var(expr), and EffectVar(args...) are assignable lvalues
            // Local() and Var() without arguments default to slot 0
            Expr::Call { callee, args, is_optional, .. } => {
                if let Expr::Variable(ref name) = *callee {
                    if !is_optional {
                        if name == "Local" && (args.len() == 0 || args.len() == 1) {
                            let index = if args.is_empty() {
                                Box::new(Expr::Literal(Literal::Int(0)))
                            } else {
                                Box::new(args.into_iter().next().unwrap())
                            };
                            return Ok(AssignmentTarget::LocalSlot(index));
                        } else if name == "Var" && (args.len() == 0 || args.len() == 1) {
                            let index = if args.is_empty() {
                                Box::new(Expr::Literal(Literal::Int(0)))
                            } else {
                                Box::new(args.into_iter().next().unwrap())
                            };
                            return Ok(AssignmentTarget::VarSlot(index));
                        } else if name == "EffectVar" {
                            // EffectVar can take any number of arguments
                            return Ok(AssignmentTarget::EffectSlot(args));
                        }
                    }
                }
                // NEW: Handle obj->LocalN("key"), obj->Local(index), obj->Var(index)
                // These are method calls that can be used as assignment targets
                else if let Expr::Property(ref object, ref method) = *callee {
                    if !is_optional {
                        if method == "LocalN" || method == "Local" || method == "Var" || method == "EffectVar" {
                            return Ok(AssignmentTarget::MethodSlot {
                                object: object.clone(),
                                method: method.clone(),
                                args,
                            });
                        }
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
            // Special case: Local(expr) and Var(expr) are valid for increment/decrement
            Expr::Call { callee, args, is_optional, .. } => {
                if let Expr::Variable(ref name) = **callee {
                    if !is_optional && args.len() == 1 && (name == "Local" || name == "Var") {
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
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> Result<Expr, ParseError> {
        // Parse the next higher-precedence level first
        let left = self.parse_or()?;

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
            _ => (false, None),
        };

        if is_assign {
            let op_token = self.consume()?;
            // Validate the left side is a legal assignment target
            let target = self.expression_to_assignment_target(left.clone(), &op_token)?;

            // Right-associative: a = b = c parses as a = (b = c)
            let value = self.parse_assignment()?;

            // Desugar compound assignments: a += b becomes a = a + b
            let final_value = match op_symbol.unwrap() {
                Symbol::Equal => value,
                Symbol::PlusEqual => Expr::Binary(Box::new(left), BinaryOp::Add, Box::new(value)),
                Symbol::MinusEqual => Expr::Binary(Box::new(left), BinaryOp::Sub, Box::new(value)),
                Symbol::StarEqual => Expr::Binary(Box::new(left), BinaryOp::Mul, Box::new(value)),
                Symbol::SlashEqual => Expr::Binary(Box::new(left), BinaryOp::Div, Box::new(value)),
                Symbol::PercentEqual => Expr::Binary(Box::new(left), BinaryOp::Mod, Box::new(value)),
                // Bitwise compound assignments
                Symbol::AndEqual => Expr::Binary(Box::new(left), BinaryOp::BitAnd, Box::new(value)),
                Symbol::OrEqual => Expr::Binary(Box::new(left), BinaryOp::BitOr, Box::new(value)),
                Symbol::XorEqual => Expr::Binary(Box::new(left), BinaryOp::BitXor, Box::new(value)),
                Symbol::LeftShiftEqual => Expr::Binary(Box::new(left), BinaryOp::LeftShift, Box::new(value)),
                Symbol::RightShiftEqual => Expr::Binary(Box::new(left), BinaryOp::RightShift, Box::new(value)),
                _ => return Err(ParseError::new(
                    format!("unknown assignment operator {:?}", op_symbol.unwrap()),
                    op_token.line,
                    op_token.column,
                )),
            };

            Ok(Expr::Assignment(target, Box::new(final_value)))
        } else {
            Ok(left)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_and()?;
        while self.consume_if_symbol(Symbol::OrOr)?.is_some() || self.consume_if_identifier("or")?.is_some() {
            let right = self.parse_and()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::Or, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_bit_or()?;
        while self.consume_if_symbol(Symbol::AndAnd)?.is_some() || self.consume_if_identifier("and")?.is_some() {
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
        let mut expr = self.parse_comparison()?;
        loop {
            if self.consume_if_symbol(Symbol::EqualEqual)?.is_some() || self.consume_if_identifier("eq")?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Equal, Box::new(right));
            } else if self.consume_if_symbol(Symbol::BangEqual)?.is_some() || self.consume_if_identifier("ne")?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::NotEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringEqual)?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::StringNotEqual)?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringNotEqual, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_shift()?;
        loop {
            if self.consume_if_symbol(Symbol::Less)?.is_some() || self.consume_if_identifier("lt")?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Less, Box::new(right));
            } else if self.consume_if_symbol(Symbol::LessEqual)?.is_some() || self.consume_if_identifier("le")?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::LessEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Greater)?.is_some() || self.consume_if_identifier("gt")?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Greater, Box::new(right));
            } else if self.consume_if_symbol(Symbol::GreaterEqual)?.is_some() || self.consume_if_identifier("ge")?.is_some() {
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
            } else if self.consume_if_symbol(Symbol::StringGreaterEqual)?.is_some() {
                let right = self.parse_shift()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::StringGreaterEqual, Box::new(right));
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

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.consume_if_symbol(Symbol::Bang)?.is_some() || self.consume_if_identifier("not")?.is_some() {
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
            } else if self.consume_if_symbol(Symbol::LBracket)?.is_some() {
                let index = self.parse_expression()?;
                self.expect_symbol(Symbol::RBracket, "expected ']' after index expression")?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.consume_if_symbol(Symbol::Dot)?.is_some() {
                let token = self.expect_identifier("expected property name after '.'")?;
                let name = if let TokenKind::Identifier(name) = token.kind {
                    name
                } else {
                    unreachable!()
                };
                expr = Expr::Property(Box::new(expr), name);
            } else if self.consume_if_symbol(Symbol::Arrow)?.is_some() {
                // Check for optional method call: ->~MethodName()
                let is_optional = self.consume_if_symbol(Symbol::Tilde)?.is_some();
                let token = self.expect_identifier("expected property/method name after '->'")? ;
                let mut name = if let TokenKind::Identifier(name) = token.kind {
                    name
                } else {
                    unreachable!()
                };

                // Check for scope resolution: ->DefID::Method
                if self.consume_if_symbol(Symbol::ColonColon)?.is_some() {
                    let method_token = self.expect_identifier("expected method name after '::'")?;
                    let method_name = if let TokenKind::Identifier(method) = method_token.kind {
                        method
                    } else {
                        unreachable!()
                    };
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
                            "'~' requires a method call: expected '(' after method name".to_string(),
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

            // Parse regular expression argument
            args.push(self.parse_expression()?);

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
            elements.push(self.parse_expression()?);
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
            let value = self.parse_expression()?;
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

    fn expect_identifier(&mut self, message: &str) -> Result<Token, ParseError> {
        let token = self.peek()?.clone();
        match token.kind {
            TokenKind::Identifier(_) => {
                self.consume()?;
                Ok(token)
            }
            _ => Err(ParseError::new(
                message.to_string(),
                token.line,
                token.column,
            )),
        }
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
        Ok(self.peeked.take().unwrap())
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

    // Check if we're looking at a context annotation after '['
    // Context annotations have the pattern:
    // - [$LocaleKey$ | ...] OR
    // - [Identifier=Value | ...] OR
    // - [Identifier | ...]
    // Distinguished from arrays by lack of commas and presence of = or | or $
    fn is_context_annotation(&mut self) -> Result<bool, ParseError> {
        // Save current position
        let saved_peeked = self.peeked.clone();

        // Check first token after '['
        let first_token = self.peek()?;
        let is_annotation = match &first_token.kind {
            // LocaleKey definitely means context annotation
            TokenKind::LocaleKey(_) => true,
            // Identifier might be context annotation if followed by = or |
            TokenKind::Identifier(_) => {
                // Consume the identifier to look at next token
                self.consume()?;
                let second_token = self.peek()?;
                // Check the second token kind and determine result
                let result = matches!(
                    &second_token.kind,
                    TokenKind::Symbol(Symbol::Equal)     // Key=Value
                    | TokenKind::Symbol(Symbol::Pipe)    // Key|...
                    | TokenKind::Symbol(Symbol::RBracket) // [Key] alone
                );
                // Note: Comma means array: [Key, ...]
                result
            }
            // Anything else is not a context annotation
            _ => false,
        };

        // Restore state
        self.peeked = saved_peeked;
        Ok(is_annotation)
    }

    fn parse_var_decl_list(&mut self, kind: VarDeclKind) -> Result<Vec<VarDecl>, ParseError> {
        // Parse: name [= expr] (, name [= expr])* ;
        let mut decls = Vec::new();

        loop {
            // Parse variable name
            let name_token = self.expect_identifier("expected variable name in declaration")?;
            let name = if let TokenKind::Identifier(name) = name_token.kind {
                name
            } else {
                unreachable!()
            };

            // Check for initializer
            let init = if self.consume_if_symbol(Symbol::Equal)?.is_some() {
                Some(self.parse_expression()?)
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

    fn parse_context_annotation_body(&mut self) -> Result<Stmt, ParseError> {
        // Context annotations are metadata for the UI system
        // Syntax: [$LocaleKey$|Property=Value|...]
        // We parse and discard these as they're not executable code
        // Note: The opening '[' has already been consumed

        // Consume all tokens until we hit the closing bracket
        loop {
            let token = self.peek()?.clone();
            match &token.kind {
                TokenKind::Symbol(Symbol::RBracket) => {
                    self.consume()?;
                    break;
                }
                TokenKind::Eof => {
                    return Err(ParseError::new(
                        "unterminated context annotation (missing ']')",
                        token.line,
                        token.column,
                    ));
                }
                _ => {
                    // Consume any token (LocaleKey, Identifier, Symbol, etc.)
                    self.consume()?;
                }
            }
        }

        // Return an empty block as context annotations have no runtime effect
        Ok(Stmt::Block(Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_script(source: &str) -> Result<Script, ParseError> {
        Parser::new(source).parse_script()
    }

    #[test]
    fn parse_return_with_simple_expression() {
        let result = parse_script("func Test() { return 42; }");
        assert!(result.is_ok());
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
