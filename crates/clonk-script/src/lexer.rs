use crate::error::ParseError;
use crate::token::{Keyword, Symbol, Token, TokenKind};
use crate::value::{
    c4_string_byte_len, c4_string_bytes, c4_string_from_bytes, c4_string_from_literal,
};

// C4AUL_MAX_String (C4Aul.h): decoded string buffer bytes.
const C4AUL_MAX_STRING: usize = 1024;
// C4AUL_MAX_Identifier (C4Aul.h): the token buffer also bounds integer text.
const C4AUL_MAX_IDENTIFIER: usize = 100;

/// Rewindable token-source state for parser lookahead that must re-lex its
/// tokens afterward. The string-operand length covers the one lexer side
/// channel that C++'s speculative `Discard` scan must not commit; diagnostics
/// intentionally remain observable.
pub(crate) struct LexerCheckpoint<'a> {
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char, usize, usize)>,
    line: usize,
    column: usize,
    just_saw_cr: bool,
    strict_level: u8,
    string_literals_len: usize,
    split_next_leading_star: bool,
}

pub struct Lexer<'a> {
    input: &'a str,
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char, usize, usize)>, // (byte_idx, char, line, column)
    line: usize,
    column: usize,
    just_saw_cr: bool,
    strict_level: u8,
    diagnostics: Vec<ParseError>,
    /// Held string operands in encounter order. The lexer contributes quoted
    /// literals and the parser adds identifier-backed map/property keys.
    string_literals: Vec<String>,
    input_is_c4_bytes: bool,
    /// C4Aul's `Shift(..., false)` recognizes one leading `*` without
    /// maximal-munching a following operator. `#appendto` uses this for its
    /// target token.
    split_next_leading_star: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        // C4Script sources are native C strings. An embedded NUL terminates
        // the source before tokenization, including when it cuts a token or
        // string literal in half.
        let input = input.split_once('\0').map_or(input, |(prefix, _)| prefix);
        Self {
            input,
            chars: input.char_indices(),
            peeked: None,
            line: 1,
            column: 1,
            just_saw_cr: false,
            strict_level: 0,
            diagnostics: Vec::new(),
            string_literals: Vec::new(),
            input_is_c4_bytes: false,
            split_next_leading_star: false,
        }
    }

    pub(crate) fn new_c4_string(input: &'a str) -> Self {
        Self {
            input_is_c4_bytes: true,
            ..Self::new(input)
        }
    }

    pub(crate) fn set_strict_level(&mut self, strict_level: u8) {
        self.strict_level = strict_level;
    }

    pub(crate) fn split_next_leading_star(&mut self) {
        self.split_next_leading_star = true;
    }

    pub(crate) fn checkpoint(&self) -> LexerCheckpoint<'a> {
        LexerCheckpoint {
            chars: self.chars.clone(),
            peeked: self.peeked,
            line: self.line,
            column: self.column,
            just_saw_cr: self.just_saw_cr,
            strict_level: self.strict_level,
            string_literals_len: self.string_literals.len(),
            split_next_leading_star: self.split_next_leading_star,
        }
    }

    pub(crate) fn restore(&mut self, checkpoint: LexerCheckpoint<'a>) {
        self.chars = checkpoint.chars;
        self.peeked = checkpoint.peeked;
        self.line = checkpoint.line;
        self.column = checkpoint.column;
        self.just_saw_cr = checkpoint.just_saw_cr;
        self.strict_level = checkpoint.strict_level;
        self.string_literals
            .truncate(checkpoint.string_literals_len);
        self.split_next_leading_star = checkpoint.split_next_leading_star;
    }

    /// A C++ `Discard` lookahead that throws does not rewind its source
    /// cursor, but it still must not retain strings scanned before the error.
    pub(crate) fn finish_failed_discard_scan(&mut self, checkpoint: LexerCheckpoint<'a>) {
        self.string_literals
            .truncate(checkpoint.string_literals_len);
    }

    /// After a non-alphanumeric byte starts a legacy ATT_IDTF, C4Aul keeps
    /// only its ordinary ASCII identifier continuation. Parameter parsing
    /// uses this after consuming a standalone `|`; bypassing normal Rust
    /// tokenization also preserves digit-leading and underscore-rich tails.
    pub(crate) fn consume_legacy_identifier_continuation(&mut self) -> String {
        let mut continuation = String::new();
        while let Some(ch) = self.peek_char() {
            if !ch.is_ascii_alphanumeric() && ch != '_' {
                break;
            }
            self.bump_char();
            continuation.push(ch);
        }
        continuation
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<ParseError> {
        std::mem::take(&mut self.diagnostics)
    }

    pub(crate) fn take_string_literals(&mut self) -> Vec<String> {
        std::mem::take(&mut self.string_literals)
    }

    /// Record a C4String operand synthesized from an identifier by the
    /// parser (map keys and property access). Native C4Aul registers these at
    /// link time just like quoted literals.
    pub(crate) fn record_string_operand(&mut self, value: String) {
        self.string_literals.push(value);
    }

    /// Static-constant strings are tokenized with C4Aul's `Ref` policy, not
    /// `Hold`. The parser calls this after recognizing that declaration so
    /// the value can instead be registered through its owning GlobalConsts
    /// C4Value.
    pub(crate) fn discard_last_string_operand(&mut self, value: &str) {
        if let Some(index) = self
            .string_literals
            .iter()
            .rposition(|candidate| candidate == value)
        {
            self.string_literals.remove(index);
        }
    }

    pub fn next_token(&mut self) -> Result<Token, ParseError> {
        loop {
            let (idx, ch, line, column) = match self.bump_char() {
                Some(info) => info,
                None => {
                    self.split_next_leading_star = false;
                    return Ok(Token::new(TokenKind::Eof, self.line, self.column));
                }
            };
            let begins_comment = ch == '/' && matches!(self.peek_char(), Some('/' | '*'));
            let split_leading_star = if ch <= ' ' || begins_comment {
                false
            } else {
                std::mem::take(&mut self.split_next_leading_star)
            };

            match ch {
                c if c <= ' ' => {
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
                    // C4Aul has exactly one `S`-prefixed operator: contiguous
                    // `S=`, and GetOperator disables it at STRICT2+. Every
                    // other spelling starts an ordinary identifier named S.
                    if self.strict_level < 2 && self.peek_char() == Some('=') {
                        self.bump_char();
                        return Ok(Token::new(
                            TokenKind::Symbol(Symbol::StringEqual),
                            line,
                            column,
                        ));
                    }
                    return self.lex_identifier('S', idx, line, column);
                }
                // C4AulParse.cpp:616-671 keeps `@` as a legacy identifier
                // initial below STRICT2; its continuation is ordinary ID text.
                '@' if self.strict_level < 2 => {
                    return self.lex_identifier(ch, idx, line, column);
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    return self.lex_identifier(ch, idx, line, column);
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
                    if split_leading_star {
                        return Ok(Token::new(TokenKind::Symbol(Symbol::Star), line, column));
                    }
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
                Some(next) if next <= ' ' => {
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
            if ch == '\n' || ch == '\r' {
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
        // C4Aul's AdvanceSpaces reaches the source NUL while still in a
        // block comment and reports ordinary EOF, not a lexer error.
        Ok(())
    }

    fn lex_identifier(
        &mut self,
        first: char,
        start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        let mut end_idx = start_idx + first.len_utf8();
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
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
            return Ok(Token::new(TokenKind::GlobalCall, line, column));
        }
        let looks_like_c4id = lexeme.len() == 4
            && lexeme
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
        let next_is_call_or_label = self
            .peek_char()
            .map(|ch| ch == '(' || (ch == ':' && self.peek_char_at(1) != Some(':')))
            .unwrap_or(false);
        if looks_like_c4id && next_is_call_or_label {
            return self.lex_stupid_func_label(lexeme.to_owned(), line, column);
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
            "nil" if self.strict_level >= 3 => TokenKind::Keyword(Keyword::Nil),
            "this" => TokenKind::Keyword(Keyword::This),
            // C4Aul type words are contextual identifiers and are recognized
            // only in parameter type position by the parser.
            // The legacy string operators `eq` and `ne` are contextual and
            // remain identifiers at the lexer level.
            _ => {
                // Check if it looks like a C4ID:
                // - Exactly 4 characters
                // - Contains only uppercase letters, digits, or underscores
                // - Not followed by '(' (function call) or ':' (label, except ::)
                if looks_like_c4id {
                    TokenKind::C4Id(lexeme.to_string())
                } else {
                    TokenKind::Identifier(lexeme.to_string())
                }
            }
        };
        Ok(Token::new(kind, line, column))
    }

    fn lex_stupid_func_label(
        &mut self,
        lexeme: String,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        let error = ParseError::new(format!("stupid func label: {lexeme}"), line, column);
        if self.strict_level >= 2 {
            Err(error)
        } else {
            self.diagnostics.push(error);
            Ok(Token::new(TokenKind::Identifier(lexeme), line, column))
        }
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
            if ch.is_ascii_alphanumeric() || ch == '_' {
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

        // C4Aul enters hexadecimal mode only for a lowercase `x`. Uppercase
        // `X` instead promotes the token to a digit-leading C4ID.
        if first == '0' {
            if let Some(ch) = self.peek_char() {
                if ch == 'x' {
                    // Consume the `x` transition character.
                    let (idx, consumed, _, _) = self.bump_char().unwrap();
                    end_idx = idx + consumed.len_utf8();

                    // Collect hex digits [0-9a-fA-F]
                    while let Some(ch) = self.peek_char() {
                        if ch.is_ascii_hexdigit() {
                            let (idx, consumed, _, _) = self.bump_char().unwrap();
                            end_idx = idx + consumed.len_utf8();
                        } else {
                            break;
                        }
                    }

                    let lexeme = &self.input[start_idx..end_idx];
                    if matches!(self.peek_char(), Some('(' | ':')) {
                        return self.lex_stupid_func_label(lexeme.to_owned(), line, column);
                    }

                    // `%SCNxPTR` accepts the bare `0x` spelling as zero on
                    // the supported C++ runtime. C4Aul copies at most
                    // C4AUL_MAX_Identifier bytes before scanning at pointer
                    // width, then intentionally truncates to C4ValueInt's
                    // signed 32 bits.
                    let lexeme = &self.input[start_idx..end_idx];
                    let lexeme = &lexeme[..lexeme.len().min(C4AUL_MAX_IDENTIFIER)];
                    let hex_slice = &lexeme[2..];
                    if hex_slice.is_empty() {
                        return Ok(Token::new_number(0, 0, true, line, column));
                    }
                    let value = u128::from_str_radix(hex_slice, 16)
                        .unwrap_or(u128::MAX)
                        .min(usize::MAX as u128) as usize;
                    return Ok(Token::new_number(
                        value as i32,
                        value as u64,
                        true,
                        line,
                        column,
                    ));
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
            // The state transition consumes its triggering character at the
            // loop footer. Once in TGS_C4ID, only later alphanumerics extend
            // the token; a second underscore terminates it.
            let (idx, consumed, _, _) = self.bump_char().unwrap();
            end_idx = idx + consumed.len_utf8();
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_alphanumeric() {
                    let (idx, consumed, _, _) = self.bump_char().unwrap();
                    end_idx = idx + consumed.len_utf8();
                } else {
                    break;
                }
            }
            let lexeme = &self.input[start_idx..end_idx];
            let next_is_call_or_label = self
                .peek_char()
                .map(|ch| ch == '(' || (ch == ':' && self.peek_char_at(1) != Some(':')))
                .unwrap_or(false);
            if next_is_call_or_label {
                return self.lex_stupid_func_label(lexeme.to_owned(), line, column);
            }
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

        // C4Aul scans the bounded token buffer with `%SCNdPTR`. Decimal
        // overflow therefore saturates at pointer-width INT_MAX instead of
        // becoming a lexer error (C4AulParse.cpp:704-743).
        let slice = &self.input[start_idx..end_idx];
        let slice = &slice[..slice.len().min(C4AUL_MAX_IDENTIFIER)];
        if matches!(self.peek_char(), Some('(' | ':')) {
            return self.lex_stupid_func_label(slice.to_owned(), line, column);
        }
        let magnitude = slice
            .parse::<u128>()
            .unwrap_or(u128::MAX)
            .min(usize::MAX as u128) as usize;
        let value = magnitude.min(isize::MAX as usize) as isize;
        Ok(Token::new_number(
            value as i32,
            magnitude as u64,
            false,
            line,
            column,
        ))
    }

    fn lex_string(
        &mut self,
        _start_idx: usize,
        line: usize,
        column: usize,
    ) -> Result<Token, ParseError> {
        let mut value = String::new();
        let mut value_is_canonical_bytes = false;
        let mut warned_too_long = false;
        while let Some((_, ch, char_line, char_column)) = self.bump_char() {
            match ch {
                '"' => {
                    let value = if value_is_canonical_bytes || self.input_is_c4_bytes {
                        c4_string_from_bytes(&c4_string_bytes(&value))
                    } else {
                        c4_string_from_literal(value)
                    };
                    self.string_literals.push(value.clone());
                    return Ok(Token::new(TokenKind::String(value), line, column));
                }
                _ if (if self.input_is_c4_bytes {
                    c4_string_byte_len(&value)
                } else {
                    value.len()
                }) >= C4AUL_MAX_STRING =>
                {
                    self.handle_string_overflow(&mut warned_too_long, char_line, char_column)?;
                }
                '\r' | '\n' => {
                    // Leave recovery after the dangling closing quote. If it
                    // were tokenized as a new opener, it could swallow this
                    // function's brace and the next top-level declaration.
                    self.skip_string_remainder();
                    return Err(ParseError::new("string not closed", char_line, char_column));
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
                other
                    if (if self.input_is_c4_bytes {
                        c4_string_byte_len(&value) + c4_string_byte_len(&other.to_string())
                    } else {
                        value.len() + other.len_utf8()
                    }) <= C4AUL_MAX_STRING =>
                {
                    value.push(other);
                }
                other => {
                    // C++ counts raw bytes and can split a UTF-8 scalar at
                    // the boundary. Preserve the prefix through the same
                    // reversible byte representation used by native strings.
                    let current_len = if self.input_is_c4_bytes {
                        c4_string_byte_len(&value)
                    } else {
                        value.len()
                    };
                    let remaining = C4AUL_MAX_STRING.saturating_sub(current_len);
                    if remaining != 0 {
                        if !value_is_canonical_bytes {
                            value = if self.input_is_c4_bytes {
                                c4_string_from_bytes(&c4_string_bytes(&value))
                            } else {
                                c4_string_from_literal(value)
                            };
                            value_is_canonical_bytes = true;
                        }
                        let bytes = if self.input_is_c4_bytes {
                            c4_string_bytes(&other.to_string())
                        } else {
                            let mut encoded = [0; 4];
                            other.encode_utf8(&mut encoded).as_bytes().to_vec()
                        };
                        value.push_str(&c4_string_from_bytes(&bytes[..remaining]));
                    }
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
            self.skip_string_remainder();
            return Err(error);
        }
        if !*warned {
            self.diagnostics.push(error);
            *warned = true;
        }
        Ok(())
    }

    fn skip_string_remainder(&mut self) {
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
        lex_all_at_strict_level(source, 0)
    }

    fn lex_all_at_strict_level(source: &str, strict_level: u8) -> Result<Vec<Token>, ParseError> {
        let mut lexer = Lexer::new(source);
        lexer.set_strict_level(strict_level);
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
    fn at_prefixed_identifier_is_legacy_only() {
        let spelling = "@legacy_42";
        for strict_level in 0..2 {
            let tokens = lex_all_at_strict_level(spelling, strict_level)
                .expect("legacy strictness accepts @-prefixed identifiers");
            assert!(matches!(
                tokens.as_slice(),
                [Token {
                    kind: TokenKind::Identifier(name),
                    ..
                }] if name == spelling
            ));
        }

        for strict_level in 2..=3 {
            let error = lex_all_at_strict_level(spelling, strict_level)
                .expect_err("modern strictness rejects @-prefixed identifiers");
            assert_eq!(error.message(), "unexpected character '@'");
        }
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
    fn nul_terminates_source_before_following_tokens() {
        let tokens = lex_all("1\0+1").expect("NUL-terminated source lexes its prefix");
        assert_eq!(
            tokens
                .into_iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![TokenKind::Number(1)]
        );
    }

    #[test]
    fn nul_inside_string_leaves_literal_unterminated() {
        let error = lex_all("\"open\0\"").expect_err("NUL cuts off the closing quote");
        assert_eq!(error.message(), "unterminated string literal");
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
    fn only_s_equal_is_a_string_operator_below_strict_two() {
        let kinds = lex_all("S= S!= S< S<= S> S>=")
            .expect("operators lex")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Symbol(Symbol::StringEqual),
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::BangEqual),
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::Less),
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::LessEqual),
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::Greater),
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::GreaterEqual),
            ]
        );

        let strict_kinds = lex_all_at_strict_level("S=1", 2)
            .expect("strict operator adjacency lexes")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            strict_kinds,
            vec![
                TokenKind::Identifier("S".into()),
                TokenKind::Symbol(Symbol::Equal),
                TokenKind::Number(1),
            ]
        );
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
    fn overlong_literal_keeps_the_first_byte_of_a_split_utf8_scalar() {
        let source = format!("\"{}\u{e9}\"", "a".repeat(C4AUL_MAX_STRING - 1));
        let tokens = lex_all(&source).expect("nonstrict overlong literal tokenizes");
        let TokenKind::String(value) = &tokens[0].kind else {
            panic!("expected string literal");
        };
        let bytes = crate::value::c4_string_bytes(value);
        assert_eq!(bytes.len(), C4AUL_MAX_STRING);
        assert_eq!(&bytes[..C4AUL_MAX_STRING - 1], vec![b'a'; 1023]);
        assert_eq!(bytes[C4AUL_MAX_STRING - 1], 0xc3);
    }

    #[test]
    fn nonstrict_full_string_ignores_a_following_line_break_like_cpp() {
        let source = format!("\"{}\n\"", "a".repeat(C4AUL_MAX_STRING));
        let tokens = lex_all(&source).expect("overflow bypasses the line-break check");
        let TokenKind::String(value) = &tokens[0].kind else {
            panic!("expected string literal");
        };
        assert_eq!(c4_string_byte_len(value), C4AUL_MAX_STRING);
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
    fn c4id_shaped_function_labels_warn_below_strict_two_and_error_at_strict_two() {
        for (source, identifier) in [
            ("CLNK(", "CLNK"),
            ("2:", "2"),
            ("0x2(", "0x2"),
            ("1A(", "1A"),
        ] {
            let mut legacy = Lexer::new(source);
            let token = legacy
                .next_token()
                .expect("legacy spelling is warning-only");
            assert_eq!(
                token.kind,
                TokenKind::Identifier(identifier.to_string()),
                "source: {source}"
            );
            assert_eq!(
                legacy.take_diagnostics()[0].message(),
                format!("stupid func label: {identifier}"),
                "source: {source}"
            );

            let mut strict = Lexer::new(source);
            strict.set_strict_level(2);
            let error = strict
                .next_token()
                .expect_err("STRICT2 rejects the same function-label spelling");
            assert_eq!(
                error.message(),
                format!("stupid func label: {identifier}"),
                "source: {source}"
            );
        }
    }

    #[test]
    fn c4_integer_literal_edges_consume_c4id_transition_underscore() {
        let kinds = lex_all("1_AA 12_A")
            .expect("transition underscores are part of digit-leading C4IDs")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::C4Id("1_AA".to_string()),
                TokenKind::C4Id("12_A".to_string()),
            ]
        );
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
    fn c4_comment_whitespace_edges_cr_ends_line_comment() {
        let kinds = lex_all("// comment\rvar x;\n")
            .expect("carriage return ends the line comment")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword(Keyword::Var),
                TokenKind::Identifier("x".to_string()),
                TokenKind::Symbol(Symbol::Semicolon),
            ]
        );
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
    fn c4_comment_whitespace_edges_unterminated_block_comment_is_eof() {
        let kinds = lex_all("var x; /* open")
            .expect("an open block comment at EOF is tolerated")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword(Keyword::Var),
                TokenKind::Identifier("x".to_string()),
                TokenKind::Symbol(Symbol::Semicolon),
            ]
        );
    }

    #[test]
    fn c4_comment_whitespace_edges_raw_newline_in_string_errors() {
        let error = lex_all("\"a\nb\"").expect_err("raw newline must not extend a string");
        assert_eq!(error.message(), "string not closed");
    }

    #[test]
    fn c4_comment_whitespace_edges_control_byte_is_whitespace() {
        let kinds = lex_all("var\u{7}x;")
            .expect("C0 control bytes are C4Aul whitespace")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword(Keyword::Var),
                TokenKind::Identifier("x".to_string()),
                TokenKind::Symbol(Symbol::Semicolon),
            ]
        );
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
    fn c4_integer_literal_edges_uppercase_x_is_a_c4id() {
        let kinds = lex_all("0XFF")
            .expect("uppercase X takes the C4ID transition")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec![TokenKind::C4Id("0XFF".to_string())]);
    }

    #[test]
    fn c4_integer_literal_edges_wrap_hex_and_decimal_to_i32() {
        // C4AulParse.cpp:704-743 scans through pointer width, including
        // overflow, and C4AulParse.cpp:3409 narrows ATT_INT to int32.
        let kinds = lex_all(
            "0xffffffff 4294967295 0xffffffffffffffffffff 99999999999999999999999 0xa0c0ff",
        )
        .expect("pointer-width integer scans truncate to C4ValueInt")
        .into_iter()
        .map(|token| token.kind)
        .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Number(-1),
                TokenKind::Number(-1),
                TokenKind::Number(-1),
                TokenKind::Number(-1),
                TokenKind::Number(0xa0c0ff),
            ]
        );
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
    fn c4_integer_literal_edges_allow_empty_hex_digits() {
        let kinds = lex_all("0x")
            .expect("bare lowercase hex prefix scans as zero")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec![TokenKind::Number(0)]);
    }
}
