use crate::ast::{AccessLevel, AppendTo, AssignmentTarget, BinaryOp, Expr, Function, Script, Stmt, UnaryOp};
use crate::error::ParseError;
use crate::lexer::Lexer;
use crate::token::{Keyword, Symbol, Token, TokenKind};
use crate::value::Literal;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    peeked: Option<Token>,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            lexer: Lexer::new(source),
            peeked: None,
        }
    }

    pub fn parse_script(&mut self) -> Result<Script, ParseError> {
        // Parse directives first (before functions)
        let mut includes = Vec::new();
        let mut appendto = None;
        let mut strict_level = None;

        while !self.is_eof()? {
            // Check if next token is a directive
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
            } else {
                // Not a directive, break to parse functions
                break;
            }
        }

        // Parse top-level local variable declarations
        while !self.is_eof()? {
            if self.consume_if_keyword(Keyword::Local)?.is_some() {
                self.parse_top_level_local_decl()?;
            } else {
                break;
            }
        }

        // Parse functions
        let mut functions = Vec::new();
        while !self.is_eof()? {
            functions.push(self.parse_function()?);
        }

        Ok(Script::with_directives(functions, includes, appendto, strict_level))
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

    fn parse_parameter_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen)? {
            return Ok(params);
        }
        loop {
            let token = self.expect_identifier("expected parameter name")?;
            if let TokenKind::Identifier(name) = token.kind {
                params.push(name);
            }
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }
        Ok(params)
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
        if self.consume_if_keyword(Keyword::Return)?.is_some() {
            return self.parse_return();
        }
        if self.consume_if_keyword(Keyword::If)?.is_some() {
            return self.parse_if();
        }
        if self.consume_if_keyword(Keyword::While)?.is_some() {
            return self.parse_while();
        }
        if self.consume_if_symbol(Symbol::LBrace)?.is_some() {
            let body = self.parse_block_statements()?;
            self.expect_symbol(Symbol::RBrace, "expected '}' to close block")?;
            return Ok(Stmt::Block(body));
        }

        // Check for context annotation: [ followed by $LocaleKey$
        // We need to look ahead to distinguish from array literals
        if self.check_symbol(Symbol::LBracket)? {
            // Consume the '['
            self.consume()?;
            // Now check if next token is a LocaleKey
            let next_token = self.peek()?;
            if matches!(next_token.kind, TokenKind::LocaleKey(_)) {
                // This is a context annotation, parse it
                return self.parse_context_annotation_body();
            } else {
                // This is an array literal, we need to handle it as an expression
                // But we already consumed the '[', so we need to parse it differently
                // For now, return an error - we'll fix array literal parsing separately
                return Err(ParseError::new(
                    "array literals in statement position not yet supported",
                    next_token.line,
                    next_token.column,
                ));
            }
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

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        if self.consume_if_symbol(Symbol::Semicolon)?.is_some() {
            return Ok(Stmt::Return(None));
        }
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
            _ => Err(ParseError::new(
                "invalid assignment target",
                eq_token.line,
                eq_token.column,
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
                // Bitwise operators (not yet in BinaryOp, but we'll add them)
                _ => return Err(ParseError::new(
                    format!("compound assignment operator {:?} not yet fully implemented", op_symbol.unwrap()),
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
        while self.consume_if_symbol(Symbol::OrOr)?.is_some() || self.consume_if_keyword(Keyword::Or)?.is_some() {
            let right = self.parse_and()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::Or, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_equality()?;
        while self.consume_if_symbol(Symbol::AndAnd)?.is_some() || self.consume_if_keyword(Keyword::And)?.is_some() {
            let right = self.parse_equality()?;
            expr = Expr::Binary(Box::new(expr), BinaryOp::And, Box::new(right));
        }
        Ok(expr)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_comparison()?;
        loop {
            if self.consume_if_symbol(Symbol::EqualEqual)?.is_some() || self.consume_if_keyword(Keyword::Eq)?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Equal, Box::new(right));
            } else if self.consume_if_symbol(Symbol::BangEqual)?.is_some() || self.consume_if_keyword(Keyword::Ne)?.is_some() {
                let right = self.parse_comparison()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::NotEqual, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_term()?;
        loop {
            if self.consume_if_symbol(Symbol::Less)?.is_some() || self.consume_if_keyword(Keyword::Lt)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Less, Box::new(right));
            } else if self.consume_if_symbol(Symbol::LessEqual)?.is_some() || self.consume_if_keyword(Keyword::Le)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::LessEqual, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Greater)?.is_some() || self.consume_if_keyword(Keyword::Gt)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Greater, Box::new(right));
            } else if self.consume_if_symbol(Symbol::GreaterEqual)?.is_some() || self.consume_if_keyword(Keyword::Ge)?.is_some() {
                let right = self.parse_term()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::GreaterEqual, Box::new(right));
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
        let mut expr = self.parse_unary()?;
        loop {
            if self.consume_if_symbol(Symbol::Star)?.is_some() {
                let right = self.parse_unary()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Mul, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Slash)?.is_some() {
                let right = self.parse_unary()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Div, Box::new(right));
            } else if self.consume_if_symbol(Symbol::Percent)?.is_some() {
                let right = self.parse_unary()?;
                expr = Expr::Binary(Box::new(expr), BinaryOp::Mod, Box::new(right));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.consume_if_symbol(Symbol::Bang)?.is_some() || self.consume_if_keyword(Keyword::Not)?.is_some() {
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
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.consume_if_symbol(Symbol::LParen)?.is_some() {
                let args = self.parse_argument_list()?;
                self.expect_symbol(Symbol::RParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
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
                let token = self.expect_identifier("expected property/method name after '->'")? ;
                let name = if let TokenKind::Identifier(name) = token.kind {
                    name
                } else {
                    unreachable!()
                };
                expr = Expr::Property(Box::new(expr), name);
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_argument_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if self.check_symbol(Symbol::RParen)? {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expression()?);
            if self.consume_if_symbol(Symbol::Comma)?.is_some() {
                continue;
            }
            break;
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.consume()?;
        match token.kind {
            TokenKind::Number(value) => Ok(Expr::Literal(Literal::Int(value))),
            TokenKind::String(value) => Ok(Expr::Literal(Literal::String(value))),
            TokenKind::Keyword(Keyword::True) => Ok(Expr::Literal(Literal::Bool(true))),
            TokenKind::Keyword(Keyword::False) => Ok(Expr::Literal(Literal::Bool(false))),
            TokenKind::Keyword(Keyword::Nil) => Ok(Expr::Literal(Literal::Nil)),
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

    fn is_eof(&mut self) -> Result<bool, ParseError> {
        let token = self.peek()?;
        Ok(matches!(token.kind, TokenKind::Eof))
    }

    fn peek(&mut self) -> Result<&Token, ParseError> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    fn consume(&mut self) -> Result<Token, ParseError> {
        if let Some(token) = self.peeked.take() {
            Ok(token)
        } else {
            self.lexer.next_token()
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

    fn parse_top_level_local_decl(&mut self) -> Result<(), ParseError> {
        // local name (, name)* ;
        // Parse first variable name
        self.expect_identifier("expected variable name after 'local'")?;

        // Parse additional comma-separated names
        while self.consume_if_symbol(Symbol::Comma)?.is_some() {
            self.expect_identifier("expected variable name after ','")?;
        }

        // Expect semicolon
        self.expect_symbol(Symbol::Semicolon, "expected ';' after local declaration")?;

        Ok(())
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
