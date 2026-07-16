use crate::error::ParseError;
use crate::token::{Keyword, Symbol, Token, TokenKind};

// C4AUL_MAX_String (C4Aul.h): decoded string buffer bytes.
const C4AUL_MAX_STRING: usize = 1024;

pub struct Lexer<'a> {
    input: &'a str,
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char, usize, usize)>, // (byte_idx, char, line, column)
    line: usize,
    column: usize,
    just_saw_cr: bool,
    strict_level: u8,
    diagnostics: Vec<ParseError>,
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
            strict_level: 0,
            diagnostics: Vec::new(),
        }
    }

    pub(crate) fn set_strict_level(&mut self, strict_level: u8) {
        self.strict_level = strict_level;
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<ParseError> {
        std::mem::take(&mut self.diagnostics)
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
                    // `.` -> Dot (property access); `...` -> Ellipsis (varargs);
                    // `..=` -> ConcatEqual; `..` -> Concat (C4Script AB_Concat).
                    if self.peek_char() == Some('.') {
                        self.bump_char();
                        let symbol = match self.peek_char() {
                            Some('.') => {
                                self.bump_char();
                                Symbol::Ellipsis
                            }
                            Some('=') => {
                                self.bump_char();
                                Symbol::ConcatEqual
                            }
                            _ => Symbol::Concat,
                        };
                        return Ok(Token::new(TokenKind::Symbol(symbol), line, column));
                    }
                    return Ok(Token::new(TokenKind::Symbol(Symbol::Dot), line, column));
                }
                '?' => {
                    // Maximal munch keeps `??`/`??=` as nil-coalescing
                    // operators; a lone `?` is strict-3 safe navigation
                    // (ATT_QMARK, C4AulParse.cpp:610-614).
                    if self.peek_char() == Some('?') {
                        self.bump_char();
                        let symbol = if self.peek_char() == Some('=') {
                            self.bump_char();
                            Symbol::QuestionQuestionEqual
                        } else {
                            Symbol::QuestionQuestion
                        };
                        return Ok(Token::new(TokenKind::Symbol(symbol), line, column));
                    }
                    return Ok(Token::new(
                        TokenKind::Symbol(Symbol::Question),
                        line,
                        column,
                    ));
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
                        if self.peek_char() == Some('=') {
                            self.bump_char();
                            return Ok(Token::new(
                                TokenKind::Symbol(Symbol::StarStarEqual),
                                line,
                                column,
                            ));
                        }
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

    /// Skip the remainder of a function-description block after its opening
    /// bracket has already been tokenized. C++ treats this as raw text and
    /// balances only `[`/`]` (C4AulParse.cpp:1825-1853).
    pub(crate) fn skip_function_description(
        &mut self,
        opening_line: usize,
        opening_column: usize,
    ) -> Result<String, ParseError> {
        let mut brackets_open = 1usize;
        let mut description = String::new();
        while let Some((_, ch, _, _)) = self.bump_char() {
            match ch {
                '[' => {
                    brackets_open += 1;
                    description.push(ch);
                }
                ']' => {
                    brackets_open -= 1;
                    if brackets_open == 0 {
                        return Ok(description);
                    }
                    description.push(ch);
                }
                _ => description.push(ch),
            }
        }

        Err(ParseError::new(
            "function desc not closed",
            opening_line,
            opening_column,
        ))
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
        if lexeme == "global"
            && self.strict_level >= 3
            && self.peek_char() == Some('-')
            && self.peek_char_at(1) == Some('>')
        {
            self.bump_char();
            self.bump_char();
            return Token::new(TokenKind::GlobalCall, line, column);
        }
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
                        .map(|ch| ch == '(' || (ch == ':' && self.peek_char_at(1) != Some(':')))
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

        // C4AulParse.cpp:711-723 changes a decimal token into TGS_C4ID
        // when the first non-digit is uppercase or `_`. Hazard's HUD ids
        // (`1HUD`, `2HUD`, `3HUD`) depend on this path.
        if matches!(self.peek_char(), Some('A'..='Z' | '_')) {
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_alphanumeric() {
                    let (idx, consumed, _, _) = self.bump_char().unwrap();
                    end_idx = idx + consumed.len_utf8();
                } else {
                    break;
                }
            }
            let lexeme = &self.input[start_idx..end_idx];
            if lexeme.len() == 4
                && lexeme
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
            {
                return Ok(Token::new(
                    TokenKind::C4Id(lexeme.to_string()),
                    line,
                    column,
                ));
            }
            return Err(ParseError::new(
                format!("invalid C4ID literal: {lexeme}"),
                line,
                column,
            ));
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
        let mut warned_too_long = false;
        while let Some((_, ch, char_line, char_column)) = self.bump_char() {
            match ch {
                '"' => {
                    return Ok(Token::new(TokenKind::String(value), line, column));
                }
                _ if value.len() >= C4AUL_MAX_STRING => {
                    self.handle_string_overflow(&mut warned_too_long, char_line, char_column)?;
                }
                '\\' => {
                    match self.peek_char() {
                        Some('"' | '\\') => {
                            let (_, escaped, _, _) = self.bump_char().unwrap();
                            value.push(escaped);
                        }
                        Some(escaped) => {
                            // C4Aul recognizes only quote and backslash. For
                            // every other sequence it emits the backslash,
                            // warns, and leaves the following character for
                            // the ordinary next tokenizer iteration.
                            value.push('\\');
                            self.diagnostics.push(ParseError::new(
                                format!("unknown escape: {escaped}"),
                                char_line,
                                char_column,
                            ));
                        }
                        None => {
                            return Err(ParseError::new(
                                "unterminated string literal",
                                line,
                                column,
                            ))
                        }
                    }
                }
                other if value.len() + other.len_utf8() <= C4AUL_MAX_STRING => {
                    value.push(other);
                }
                _ => {
                    // C++ counts raw bytes and can split a UTF-8 scalar at
                    // the boundary. Rust strings cannot; retain whole scalars
                    // while enforcing the same byte-length ceiling.
                    self.handle_string_overflow(&mut warned_too_long, char_line, char_column)?;
                }
            }
        }
        Err(ParseError::new("unterminated string literal", line, column))
    }

    fn handle_string_overflow(
        &mut self,
        warned: &mut bool,
        line: usize,
        column: usize,
    ) -> Result<(), ParseError> {
        let error = ParseError::new("string too long", line, column);
        if self.strict_level >= 3 {
            // Once the C++ buffer is full, escape handling is bypassed: the
            // first following quote closes the token even after a backslash.
            self.skip_overlong_string_remainder();
            return Err(error);
        }
        if !*warned {
            self.diagnostics.push(error);
            *warned = true;
        }
        Ok(())
    }

    fn skip_overlong_string_remainder(&mut self) {
        while let Some((_, ch, _, _)) = self.bump_char() {
            if ch == '"' {
                break;
            }
        }
    }

    fn lex_locale_key(
        &mut self,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        // We've already consumed the opening '$'
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
                _ => {}
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
        assert!(!tokens.is_empty());
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
    fn lone_question_and_nil_coalescing_tokens_remain_distinct() {
        let tokens = lex_all("? ?? ??=").expect("question operators lex");
        assert_eq!(
            tokens
                .into_iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                TokenKind::Symbol(Symbol::Question),
                TokenKind::Symbol(Symbol::QuestionQuestion),
                TokenKind::Symbol(Symbol::QuestionQuestionEqual),
            ]
        );
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
    fn tokenizes_c4ids_starting_with_a_digit_like_cpp() {
        // C4AulParse.cpp:711-723 promotes a decimal token to TGS_C4ID
        // when its next character is uppercase or underscore.
        let tokens = lex_all("1HUD 2HUD 3HUD").expect("numeric-leading C4IDs lex");
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                TokenKind::C4Id("1HUD".to_string()),
                TokenKind::C4Id("2HUD".to_string()),
                TokenKind::C4Id("3HUD".to_string()),
            ]
        );
    }

    #[test]
    fn digit_leading_c4id_does_not_consume_an_underscore_like_cpp() {
        // TGS_Int enters TGS_C4ID when it sees `_`, but TGS_C4ID itself
        // consumes only ASCII letters and digits; the partial token then
        // fails LooksLikeID (C4AulParse.cpp:711-723,747-763). Preserve that
        // historical asymmetry instead of accepting a new identifier form.
        assert!(lex_all("1_AA").is_err());
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
            assert_eq!(n, 0xabcd12);
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
