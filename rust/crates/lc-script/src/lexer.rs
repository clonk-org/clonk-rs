use crate::error::ParseError;
use crate::token::{Keyword, Symbol, Token, TokenKind};

pub struct Lexer<'a> {
    input: &'a str,
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char, usize, usize)>, // (byte_idx, char, line, column)
    line: usize,
    column: usize,
    just_saw_cr: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices(),
            peeked: None,
            line: 1,
            column: 1,
            just_saw_cr: false,
        }
    }

    pub fn next_token(&mut self) -> Result<Token, ParseError> {
        loop {
            let (idx, ch, line, column) = match self.bump_char() {
                Some(info) => info,
                None => {
                    return Ok(Token::new(TokenKind::Eof, self.line, self.column));
                }
            };

            match ch {
                c if c.is_whitespace() => {
                    self.consume_whitespace(c);
                    continue;
                }
                '/' => {
                    if self.peek_char() == Some('/') {
                        self.consume_line_comment();
                        continue;
                    } else if self.peek_char() == Some('*') {
                        self.consume_block_comment()?;
                        continue;
                    } else if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::SlashEqual),
                            line,
                            column,
                        ));
                    } else {
                        return Ok(Token::new(TokenKind::Symbol(Symbol::Slash), line, column));
                    }
                }
                'S' => {
                    // String comparison operators: S=, S!=, S<, S<=, S>, S>=
                    match self.peek_char() {
                        Some('=') => {
                            self.bump_char();
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::StringEqual),
                                line,
                                column,
                            ));
                        }
                        Some('!') => {
                            self.bump_char();
                            if self.peek_char() == Some('=') {
                                self.bump_char();
                                return Ok(Token::new(
                                    TokenKind::Symbol(Symbol::StringNotEqual),
                                    line,
                                    column,
                                ));
                            }
                            return Err(ParseError::new(
                                "expected '=' after 'S!' in string comparison operator".to_string(),
                                line,
                                column,
                            ));
                        }
                        Some('<') => {
                            self.bump_char();
                            if self.peek_char() == Some('=') {
                                self.bump_char();
                                return Ok(Token::new(
                                    TokenKind::Symbol(Symbol::StringLessEqual),
                                    line,
                                    column,
                                ));
                            }
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::StringLess),
                                line,
                                column,
                            ));
                        }
                        Some('>') => {
                            self.bump_char();
                            if self.peek_char() == Some('=') {
                                self.bump_char();
                                return Ok(Token::new(
                                    TokenKind::Symbol(Symbol::StringGreaterEqual),
                                    line,
                                    column,
                                ));
                            }
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::StringGreater),
                                line,
                                column,
                            ));
                        }
                        _ => {
                            // 'S' alone is an identifier, let it fall through to identifier lexing
                            return Ok(self.lex_identifier('S', idx, line, column));
                        }
                    }
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    return Ok(self.lex_identifier(ch, idx, line, column));
                }
                '0'..='9' => {
                    return self.lex_number(ch, idx, line, column);
                }
                '"' => {
                    return self.lex_string(idx, line, column);
                }
                '#' => {
                    return Ok(self.lex_directive(ch, idx, line, column));
                }
                '(' => return Ok(Token::new(TokenKind::Symbol(Symbol::LParen), line, column)),
                ')' => return Ok(Token::new(TokenKind::Symbol(Symbol::RParen), line, column)),
                '{' => return Ok(Token::new(TokenKind::Symbol(Symbol::LBrace), line, column)),
                '}' => return Ok(Token::new(TokenKind::Symbol(Symbol::RBrace), line, column)),
                '[' => {
                    return Ok(Token::new(
                        TokenKind::Symbol(Symbol::LBracket),
                        line,
                        column,
                    ))
                }
                ']' => {
                    return Ok(Token::new(
                        TokenKind::Symbol(Symbol::RBracket),
                        line,
                        column,
                    ))
                }
                ',' => return Ok(Token::new(TokenKind::Symbol(Symbol::Comma), line, column)),
                ';' => {
                    return Ok(Token::new(
                        TokenKind::Symbol(Symbol::Semicolon),
                        line,
                        column,
                    ))
                }
                ':' => {
                    // Check for :: (scope resolution)
                    if self.peek_char() == Some(':') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::ColonColon),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Colon), line, column));
                }
                '.' => {
                    // Check for ... (ellipsis)
                    if self.peek_char() == Some('.') {
                        self.bump_char();
                        if self.peek_char() == Some('.') {
                            self.bump_char();
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::Ellipsis),
                                line,
                                column,
                            ));
                        } else {
                            // Two dots is an error - not valid in C4Script
                            return Err(ParseError::new(
                                "unexpected '..' - use '...' for varargs forwarding or '.' for property access".to_string(),
                                line,
                                column,
                            ));
                        }
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Dot), line, column));
                }
                '+' => {
                    if self.peek_char() == Some('+') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::PlusPlus),
                            line,
                            column,
                        ));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::PlusEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Plus), line, column));
                }
                '-' => {
                    if self.peek_char() == Some('>') {
                        self.bump_char();
                        return Ok(Token::new(TokenKind::Symbol(Symbol::Arrow), line, column));
                    }
                    if self.peek_char() == Some('-') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::MinusMinus),
                            line,
                            column,
                        ));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::MinusEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Minus), line, column));
                }
                '*' => {
                    if self.peek_char() == Some('*') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::StarStar),
                            line,
                            column,
                        ));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::StarEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Star), line, column));
                }
                '%' => {
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::PercentEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Percent), line, column));
                }
                '=' => {
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::EqualEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Equal), line, column));
                }
                '!' => {
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::BangEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Bang), line, column));
                }
                '<' => {
                    if self.peek_char() == Some('<') {
                        self.bump_char();
                        if self.peek_char() == Some('=') {
                            self.bump_char();
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::LeftShiftEqual),
                                line,
                                column,
                            ));
                        }
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::LeftShift),
                            line,
                            column,
                        ));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::LessEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Less), line, column));
                }
                '>' => {
                    if self.peek_char() == Some('>') {
                        self.bump_char();
                        if self.peek_char() == Some('=') {
                            self.bump_char();
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::RightShiftEqual),
                                line,
                                column,
                            ));
                        }
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::RightShift),
                            line,
                            column,
                        ));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::GreaterEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Greater), line, column));
                }
                '&' => {
                    if self.peek_char() == Some('&') {
                        self.bump_char();
                        return Ok(Token::new(TokenKind::Symbol(Symbol::AndAnd), line, column));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::AndEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(
                        TokenKind::Symbol(Symbol::Ampersand),
                        line,
                        column,
                    ));
                }
                '|' => {
                    if self.peek_char() == Some('|') {
                        self.bump_char();
                        return Ok(Token::new(TokenKind::Symbol(Symbol::OrOr), line, column));
                    }
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(TokenKind::Symbol(Symbol::OrEqual), line, column));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Pipe), line, column));
                }
                '^' => {
                    if self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::XorEqual),
                            line,
                            column,
                        ));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Caret), line, column));
                }
                '~' => {
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Tilde), line, column));
                }
                '\r' | '\n' => {
                    // newline already processed in bump_char
                    continue;
                }
                '$' => {
                    return self.lex_locale_key(idx, line, column);
                }
                _ => {
                    return Err(ParseError::new(
                        format!("unexpected character '{ch}'"),
                        line,
                        column,
                    ));
                }
            }
        }
    }

    fn bump_char(&mut self) -> Option<(usize, char, usize, usize)> {
        if let Some((idx, ch, line, column)) = self.peeked.take() {
            // Return cached character with its original position
            return Some((idx, ch, line, column));
        }
        // Read fresh character
        let (idx, ch) = self.chars.next()?;
        let line = self.line;
        let column = self.column;
        self.advance_position(ch);
        Some((idx, ch, line, column))
    }

    fn peek_char(&mut self) -> Option<char> {
        if self.peeked.is_none() {
            // Read and cache character with position
            let (idx, ch) = self.chars.next()?;
            let line = self.line;
            let column = self.column;
            self.advance_position(ch);
            self.peeked = Some((idx, ch, line, column));
        }
        self.peeked.map(|(_, ch, _, _)| ch)
    }

    fn peek_char_at(&mut self, offset: usize) -> Option<char> {
        // Get current position in the input
        let current_idx = if let Some((idx, _, _, _)) = self.peeked {
            idx
        } else {
            self.chars.clone().next().map(|(idx, _)| idx)?
        };

        // Calculate target position
        let mut chars_iter = self.input[current_idx..].chars();
        for _ in 0..offset {
            chars_iter.next()?;
        }
        chars_iter.next()
    }

    fn advance_position(&mut self, ch: char) {
        match ch {
            '\r' => {
                self.line += 1;
                self.column = 1;
                self.just_saw_cr = true;
            }
            '\n' => {
                if !self.just_saw_cr {
                    self.line += 1;
                }
                self.column = 1;
                self.just_saw_cr = false;
            }
            _ => {
                self.column += 1;
                self.just_saw_cr = false;
            }
        }
    }

    fn consume_whitespace(&mut self, ch: char) {
        let mut current = ch;
        loop {
            match current {
                '\n' | '\r' => {
                    // already handled by advance_position
                }
                _ => {}
            }
            match self.peek_char() {
                Some(next) if next.is_whitespace() => {
                    current = next;
                    self.bump_char();
                }
                _ => break,
            }
        }
    }

    fn consume_line_comment(&mut self) {
        self.bump_char(); // consume second '/'
        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.bump_char();
        }
    }

    fn consume_block_comment(&mut self) -> Result<(), ParseError> {
        self.bump_char(); // consume '*'
        while let Some((_, ch, _line, _column)) = self.bump_char() {
            if ch == '*' && self.peek_char() == Some('/') {
                self.bump_char();
                return Ok(());
            }
            if ch == '\r' && self.peek_char() == Some('\n') {
                self.bump_char();
            }
        }
        Err(ParseError::new(
            "unterminated block comment",
            self.line,
            self.column,
        ))
    }

    fn lex_identifier(
        &mut self,
        first: char,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Token {
        let mut end_idx = start_idx + first.len_utf8();
        while let Some(ch) = self.peek_char() {
            if ch.is_alphanumeric() || ch == '_' {
                let (idx, consumed, _, _) = self.bump_char().unwrap();
                end_idx = idx + consumed.len_utf8();
            } else {
                break;
            }
        }
        let lexeme = &self.input[start_idx..end_idx];
        let kind = match lexeme {
            "func" => TokenKind::Keyword(Keyword::Func),
            "global" => TokenKind::Keyword(Keyword::Global),
            "private" => TokenKind::Keyword(Keyword::Private),
            "protected" => TokenKind::Keyword(Keyword::Protected),
            "public" => TokenKind::Keyword(Keyword::Public),
            "local" => TokenKind::Keyword(Keyword::Local),
            "var" => TokenKind::Keyword(Keyword::Var),
            "static" => TokenKind::Keyword(Keyword::Static),
            "const" => TokenKind::Keyword(Keyword::Const),
            "if" => TokenKind::Keyword(Keyword::If),
            "else" => TokenKind::Keyword(Keyword::Else),
            "while" => TokenKind::Keyword(Keyword::While),
            "for" => TokenKind::Keyword(Keyword::For),
            "in" => TokenKind::Keyword(Keyword::In),
            "return" => TokenKind::Keyword(Keyword::Return),
            "break" => TokenKind::Keyword(Keyword::Break),
            "continue" => TokenKind::Keyword(Keyword::Continue),
            "true" => TokenKind::Keyword(Keyword::True),
            "false" => TokenKind::Keyword(Keyword::False),
            "nil" => TokenKind::Keyword(Keyword::Nil),
            "this" => TokenKind::Keyword(Keyword::This),
            // Type keywords are contextual - treated as identifiers here,
            // recognized as keywords only in type annotation contexts by parser
            // "int", "bool", "string", "object", "id", "array", "proplist", "effect"
            // Keyword operators are also contextual - treated as identifiers here,
            // recognized as operators in expression contexts by parser
            // "eq", "ne", "lt", "le", "gt", "ge", "and", "or", "not"
            _ => {
                // Check if it looks like a C4ID:
                // - Exactly 4 characters
                // - Contains only uppercase letters, digits, or underscores
                // - Not followed by '(' (function call) or ':' (label, except ::)
                if lexeme.len() == 4
                    && lexeme
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                {
                    // Check what follows the identifier
                    let next_is_call_or_label = self
                        .peek_char()
                        .map(|ch| {
                            ch == '('
                                || (ch == ':'
                                    && self.peek_char_at(1).map_or(true, |ch2| ch2 != ':'))
                        })
                        .unwrap_or(false);

                    if !next_is_call_or_label {
                        TokenKind::C4Id(lexeme.to_string())
                    } else {
                        TokenKind::Identifier(lexeme.to_string())
                    }
                } else {
                    TokenKind::Identifier(lexeme.to_string())
                }
            }
        };
        Token::new(kind, line, column)
    }

    fn lex_directive(
        &mut self,
        first: char,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Token {
        // Start with '#', continue reading alphanumeric chars to form directive
        let mut end_idx = start_idx + first.len_utf8();
        while let Some(ch) = self.peek_char() {
            if ch.is_alphanumeric() || ch == '_' {
                let (idx, consumed, _, _) = self.bump_char().unwrap();
                end_idx = idx + consumed.len_utf8();
            } else {
                break;
            }
        }
        let lexeme = &self.input[start_idx..end_idx];
        // Return the full directive string including '#'
        Token::new(TokenKind::Directive(lexeme.to_string()), line, column)
    }

    fn lex_number(
        &mut self,
        first: char,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        let mut end_idx = start_idx + first.len_utf8();

        // Check for hexadecimal literal: 0x or 0X
        if first == '0' {
            if let Some(ch) = self.peek_char() {
                if ch == 'x' || ch == 'X' {
                    // Consume the 'x' or 'X'
                    let (idx, consumed, _, _) = self.bump_char().unwrap();
                    end_idx = idx + consumed.len_utf8();

                    // Collect hex digits [0-9a-fA-F]
                    let hex_start = end_idx;
                    while let Some(ch) = self.peek_char() {
                        if ch.is_ascii_hexdigit() {
                            let (idx, consumed, _, _) = self.bump_char().unwrap();
                            end_idx = idx + consumed.len_utf8();
                        } else {
                            break;
                        }
                    }

                    // Check if we actually got any hex digits
                    if end_idx == hex_start {
                        return Err(ParseError::new(
                            "hexadecimal literal has no digits".to_string(),
                            line,
                            column,
                        ));
                    }

                    // Parse hex digits (skip "0x" prefix)
                    let hex_slice = &self.input[hex_start..end_idx];
                    match i32::from_str_radix(hex_slice, 16) {
                        Ok(value) => return Ok(Token::new(TokenKind::Number(value), line, column)),
                        Err(_) => {
                            return Err(ParseError::new(
                                format!("hexadecimal literal out of range: 0x{hex_slice}"),
                                line,
                                column,
                            ))
                        }
                    }
                }
            }
        }

        // Regular decimal number parsing
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() {
                let (idx, consumed, _, _) = self.bump_char().unwrap();
                end_idx = idx + consumed.len_utf8();
            } else {
                break;
            }
        }
        let slice = &self.input[start_idx..end_idx];
        match slice.parse::<i32>() {
            Ok(value) => Ok(Token::new(TokenKind::Number(value), line, column)),
            Err(_) => Err(ParseError::new(
                format!("integer literal out of range: {slice}"),
                line,
                column,
            )),
        }
    }

    fn lex_string(
        &mut self,
        _start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        let mut value = String::new();
        while let Some((_, ch, _, _)) = self.bump_char() {
            match ch {
                '"' => {
                    return Ok(Token::new(TokenKind::String(value), line, column));
                }
                '\\' => {
                    if let Some((_, escaped, _, _)) = self.bump_char() {
                        match escaped {
                            'n' => value.push('\n'),
                            'r' => value.push('\r'),
                            't' => value.push('\t'),
                            '"' => value.push('"'),
                            '\\' => value.push('\\'),
                            other => {
                                return Err(ParseError::new(
                                    format!("unknown escape sequence \\{other}"),
                                    line,
                                    column,
                                ));
                            }
                        }
                    } else {
                        return Err(ParseError::new("unterminated string literal", line, column));
                    }
                }
                other => value.push(other),
            }
        }
        Err(ParseError::new("unterminated string literal", line, column))
    }

    fn lex_locale_key(
        &mut self,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        // We've already consumed the opening '$'
        let mut end_idx = start_idx + 1; // skip the opening '$'

        while let Some((idx, ch, _, _)) = self.bump_char() {
            match ch {
                '$' => {
                    // Found closing '$', extract the key without the $ delimiters
                    let key = &self.input[start_idx + 1..idx];
                    return Ok(Token::new(
                        TokenKind::LocaleKey(key.to_string()),
                        line,
                        column,
                    ));
                }
                '\n' | '\r' => {
                    return Err(ParseError::new(
                        "unterminated localization key (missing closing '$')",
                        line,
                        column,
                    ));
                }
                _ => {
                    end_idx = idx + ch.len_utf8();
                }
            }
        }

        Err(ParseError::new(
            "unterminated localization key (missing closing '$')",
            line,
            column,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_all(source: &str) -> Result<Vec<Token>, ParseError> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next_token()?;
            if matches!(token.kind, TokenKind::Eof) {
                break;
            }
            tokens.push(token);
        }
        Ok(tokens)
    }

    #[test]
    fn tokenizes_private_keyword() {
        let tokens = lex_all("private").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(
            tokens[0].kind,
            TokenKind::Keyword(Keyword::Private)
        ));
    }

    #[test]
    fn tokenizes_protected_keyword() {
        let tokens = lex_all("protected").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(
            tokens[0].kind,
            TokenKind::Keyword(Keyword::Protected)
        ));
    }

    #[test]
    fn tokenizes_public_keyword() {
        let tokens = lex_all("public").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(
            tokens[0].kind,
            TokenKind::Keyword(Keyword::Public)
        ));
    }

    #[test]
    fn tokenizes_global_keyword() {
        let tokens = lex_all("global").unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(matches!(
            tokens[0].kind,
            TokenKind::Keyword(Keyword::Global)
        ));
    }

    #[test]
    fn handles_windows_line_endings() {
        let source = "var x = 1;\r\nvar y = 2;\r\n";
        let tokens = lex_all(source).unwrap();
        // Should tokenize successfully despite \r\n endings
        assert!(tokens.len() > 0);
        // Check that the second var is on line 2
        let var_positions: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Keyword(Keyword::Var)))
            .collect();
        assert_eq!(var_positions.len(), 2);
        assert_eq!(var_positions[0].line, 1);
        assert_eq!(var_positions[1].line, 2);
    }

    #[test]
    fn tracks_line_numbers_correctly() {
        let source = "var a = 1;\nvar b = 2;\nvar c = 3;";
        let tokens = lex_all(source).unwrap();
        let var_positions: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Keyword(Keyword::Var)))
            .collect();
        assert_eq!(var_positions.len(), 3);
        assert_eq!(var_positions[0].line, 1);
        assert_eq!(var_positions[1].line, 2);
        assert_eq!(var_positions[2].line, 3);
    }

    #[test]
    fn tracks_column_numbers_correctly() {
        let source = "var a = 1;";
        let tokens = lex_all(source).unwrap();
        // var starts at column 1
        assert_eq!(tokens[0].column, 1);
        // 'a' starts at column 5
        assert_eq!(tokens[1].column, 5);
    }

    #[test]
    fn tokenizes_operators() {
        let source = "+ - * / % == != < <= > >= && || !";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 14);
    }

    #[test]
    fn tokenizes_string_literals() {
        let source = r#""hello world""#;
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::String(s) = &tokens[0].kind {
            assert_eq!(s, "hello world");
        } else {
            panic!("Expected string literal");
        }
    }

    #[test]
    fn tokenizes_integer_literals() {
        let source = "42";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::Number(n) = tokens[0].kind {
            assert_eq!(n, 42);
        } else {
            panic!("Expected integer literal");
        }
    }

    #[test]
    fn tokenizes_identifiers() {
        let source = "myVar myFunc";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 2);
        if let TokenKind::Identifier(id) = &tokens[0].kind {
            assert_eq!(id, "myVar");
        } else {
            panic!("Expected identifier");
        }
    }

    #[test]
    fn skips_line_comments() {
        let source = "var x = 1; // this is a comment\nvar y = 2;";
        let tokens = lex_all(source).unwrap();
        let var_count = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Keyword(Keyword::Var)))
            .count();
        assert_eq!(var_count, 2);
    }

    #[test]
    fn skips_block_comments() {
        let source = "var x = 1; /* this is\na block comment */ var y = 2;";
        let tokens = lex_all(source).unwrap();
        let var_count = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Keyword(Keyword::Var)))
            .count();
        assert_eq!(var_count, 2);
    }

    #[test]
    fn tokenizes_hex_literal_lowercase() {
        let source = "0xa0c0ff";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::Number(n) = tokens[0].kind {
            assert_eq!(n, 0xa0c0ff);
        } else {
            panic!(
                "Expected hex literal to be tokenized as Number, got {:?}",
                tokens[0].kind
            );
        }
    }

    #[test]
    fn tokenizes_hex_literal_uppercase_x() {
        let source = "0XFF";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::Number(n) = tokens[0].kind {
            assert_eq!(n, 0xFF);
        } else {
            panic!("Expected hex literal to be tokenized as Number");
        }
    }

    #[test]
    fn tokenizes_hex_literal_mixed_case() {
        let source = "0xAbCd12";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::Number(n) = tokens[0].kind {
            assert_eq!(n, 0xAbCd12);
        } else {
            panic!("Expected hex literal to be tokenized as Number");
        }
    }

    #[test]
    fn tokenizes_hex_in_function_call() {
        let source = r#"CreateParticle("Test", 0, 0, 0, 0, 30, 0xa0c0ff)"#;
        let result = lex_all(source);
        assert!(result.is_ok());
        let tokens = result.unwrap();
        // Should have: identifier, lparen, string, comma, number, comma, number, comma, number, comma, number, comma, number, comma, hex, rparen
        // Find the last number token (should be the hex literal)
        let number_tokens: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Number(_)))
            .collect();
        // Should have 5 number tokens: 0, 0, 0, 0, 30, 0xa0c0ff
        assert_eq!(number_tokens.len(), 6);
        if let TokenKind::Number(n) = number_tokens[5].kind {
            assert_eq!(n, 0xa0c0ff);
        }
    }

    #[test]
    fn hex_literal_zero() {
        let source = "0x0";
        let tokens = lex_all(source).unwrap();
        assert_eq!(tokens.len(), 1);
        if let TokenKind::Number(n) = tokens[0].kind {
            assert_eq!(n, 0);
        } else {
            panic!("Expected hex literal to be tokenized as Number");
        }
    }

    #[test]
    fn hex_literal_no_digits_error() {
        let source = "0x";
        let result = lex_all(source);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.message().contains("no digits"));
        }
    }
}
