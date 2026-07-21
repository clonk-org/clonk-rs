//! RTF plain-text extraction, mirroring `C4RTFFile::GetPlainText`
//! (src/C4RTF.cpp:244-304) and its keyword table (:311-362). The scenario
//! selection dialog feeds `Desc??.rtf` components through this to fill the
//! right book page (C4StartupScenSelDlg.cpp:523-531).

/// Destination state of the current group (C4RTF.h dsNormal/dsSkip).
#[derive(Clone, Copy, PartialEq)]
enum Dest {
    Normal,
    Skip,
}

/// Parser sub-state (C4RTF.h psNormal/psBinary/psHex).
#[derive(Clone, Copy, PartialEq)]
enum ParseState {
    Normal,
    Binary,
    Hex,
}

#[derive(Clone)]
struct PropertyState {
    dest: Dest,
    state: ParseState,
    hex_bin_count: usize,
    hex_byte: u8,
}

impl Default for PropertyState {
    fn default() -> Self {
        Self {
            dest: Dest::Normal,
            state: ParseState::Normal,
            hex_bin_count: 0,
            hex_byte: 0,
        }
    }
}

enum Keyword {
    /// Emits literal characters (C4RTF.cpp kwdChars).
    Chars(&'static str),
    /// Switches the group destination (kwdDest); always to skip here.
    SkipDest,
    /// `\bin`: raw byte passthrough of the parameter length (specBin).
    Bin,
    /// `\*`: skip the destination if the next keyword is unknown
    /// (specSkipDest).
    SkipIfUnknown,
    /// `\'`: two hex digits follow (specHex).
    Hex,
}

/// The keyword table of C4RTF.cpp:311-362 reduced to the actions the plain-
/// text pass needs (property keywords are ignored there as well).
fn lookup_keyword(keyword: &str) -> Option<Keyword> {
    match keyword {
        // NOTE: C++ maps \tab to "\n" as well (C4RTF.cpp:317).
        "par" | "tab" => Some(Keyword::Chars("\n")),
        "ldblquote" | "rdblquote" => Some(Keyword::Chars("\"")),
        "lquote" | "rquote" => Some(Keyword::Chars("'")),
        "bin" => Some(Keyword::Bin),
        "*" => Some(Keyword::SkipIfUnknown),
        "'" => Some(Keyword::Hex),
        "author" | "buptim" | "colortbl" | "comment" | "creatim" | "doccomm" | "fonttbl"
        | "footer" | "footerf" | "footerl" | "footerr" | "footnote" | "ftncn" | "ftnsep"
        | "ftnsepc" | "header" | "headerf" | "headerl" | "headerr" | "info" | "keywords"
        | "operator" | "pict" | "printim" | "private1" | "revtim" | "rxe" | "stylesheet"
        | "subject" | "tc" | "title" | "txe" | "xe" => Some(Keyword::SkipDest),
        "{" => Some(Keyword::Chars("{")),
        "}" => Some(Keyword::Chars("}")),
        "\\" => Some(Keyword::Chars("\\")),
        _ => None,
    }
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    stack: Vec<PropertyState>,
    skip_dest_if_unknown: bool,
    /// Output bytes in the source (Windows-1252) charset, decoded at the end.
    out: Vec<u8>,
}

struct ParserError(&'static str);

impl<'a> Parser<'a> {
    fn state(&mut self) -> &mut PropertyState {
        self.stack.last_mut().expect("state stack never empty")
    }

    fn emit(&mut self, bytes: &[u8]) {
        if self.stack.last().expect("state").dest == Dest::Normal {
            self.out.extend_from_slice(bytes);
        }
    }

    /// C4RTFFile::ParseKeyword (C4RTF.cpp:117-178).
    fn parse_keyword(&mut self) -> Result<(), ParserError> {
        let mut keyword = String::new();
        let mut param: i32 = 0;
        let mut has_param = false;
        let Some(&first) = self.data.get(self.pos) else {
            return Err(ParserError("Unexpected end of file")); // AssertNoEOF
        };
        self.pos += 1;
        if !first.is_ascii_alphabetic() {
            keyword.push(first as char);
        } else {
            let mut c = first;
            loop {
                if keyword.len() < 30 {
                    keyword.push(c as char);
                }
                match self.data.get(self.pos) {
                    Some(&next) if next.is_ascii_alphabetic() => {
                        c = next;
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
            let mut c = match self.data.get(self.pos) {
                Some(&next) => {
                    self.pos += 1;
                    next
                }
                None => 0,
            };
            let mut sign = 1;
            if c == b'-' {
                sign = -1;
                if let Some(&next) = self.data.get(self.pos) {
                    c = next;
                    self.pos += 1;
                }
            }
            if c.is_ascii_digit() {
                let mut digits = String::new();
                loop {
                    if digits.len() < 20 {
                        digits.push(c as char);
                    }
                    match self.data.get(self.pos) {
                        Some(&next) if next.is_ascii_digit() => {
                            c = next;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                if let Some(&next) = self.data.get(self.pos) {
                    c = next;
                    self.pos += 1;
                } else {
                    c = 0;
                }
                param = digits.parse::<i32>().unwrap_or(0) * sign;
                has_param = true;
            }
            // A non-space delimiter does not belong to the keyword.
            if c != b' ' && self.pos > 0 {
                self.pos -= 1;
            }
        }
        self.translate_keyword(&keyword, param, has_param);
        Ok(())
    }

    /// C4RTFFile::TranslateKeyword (C4RTF.cpp:80-115).
    fn translate_keyword(&mut self, keyword: &str, param: i32, _has_param: bool) {
        let Some(action) = lookup_keyword(keyword) else {
            if self.skip_dest_if_unknown {
                self.state().dest = Dest::Skip;
                self.skip_dest_if_unknown = false;
            }
            return;
        };
        self.skip_dest_if_unknown = false;
        match action {
            Keyword::Chars(chars) => self.emit(chars.as_bytes()),
            Keyword::SkipDest => {
                if self.state().dest != Dest::Skip {
                    self.state().dest = Dest::Skip;
                }
            }
            Keyword::Bin => {
                if param > 0 {
                    self.state().state = ParseState::Binary;
                    self.state().hex_bin_count = param as usize;
                }
            }
            Keyword::SkipIfUnknown => self.skip_dest_if_unknown = true,
            Keyword::Hex => {
                self.state().state = ParseState::Hex;
                self.state().hex_bin_count = 2;
            }
        }
    }

    /// C4RTFFile::ParseHexChar (C4RTF.cpp:204-220).
    fn parse_hex_char(&mut self, c: u8) -> Result<(), ParserError> {
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return Err(ParserError("Invalid hex character")),
        };
        let state = self.state();
        state.hex_byte = (state.hex_byte << 4) | digit;
        state.hex_bin_count -= 1;
        if state.hex_bin_count == 0 {
            state.state = ParseState::Normal;
            let byte = state.hex_byte;
            self.emit(&[byte]);
        }
        Ok(())
    }

    /// C4RTFFile::GetPlainText main loop (C4RTF.cpp:244-304).
    fn run(&mut self) -> Result<(), ParserError> {
        while let Some(&c) = self.data.get(self.pos) {
            self.pos += 1;
            if self.state().state == ParseState::Binary {
                self.state().hex_bin_count -= 1;
                if self.state().hex_bin_count == 0 {
                    self.state().state = ParseState::Normal;
                }
                self.emit(&[c]);
                continue;
            }
            match c {
                b'{' => {
                    let mut new_state = self.stack.last().expect("state").clone();
                    new_state.state = ParseState::Normal;
                    self.stack.push(new_state);
                }
                b'}' => {
                    if self.stack.len() < 2 {
                        return Err(ParserError("Too many brackets closed"));
                    }
                    self.stack.pop();
                    self.state().state = ParseState::Normal;
                }
                b'\\' => self.parse_keyword()?,
                0x0d | 0x0a => {} // ignored chars
                _ => match self.state().state {
                    ParseState::Normal => self.emit(&[c]),
                    ParseState::Hex => self.parse_hex_char(c)?,
                    ParseState::Binary => unreachable!("handled above"),
                },
            }
        }
        // all states must be closed in the end
        if self.stack.len() > 1 {
            return Err(ParserError("Block not closed"));
        }
        Ok(())
    }
}

/// Extracts plain text from RTF bytes like `C4RTFFile::GetPlainText`; the
/// legacy byte output (Windows-1252) is decoded to UTF-8. Invalid RTF yields
/// an error string like the C++ version (C4RTF.cpp:294-299).
pub fn rtf_to_plain_text(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let mut parser = Parser {
        data,
        pos: 0,
        stack: vec![PropertyState::default()],
        skip_dest_if_unknown: false,
        out: Vec::new(),
    };
    match parser.run() {
        Ok(()) => crate::scenario::decode_legacy_text(&parser.out),
        Err(ParserError(detail)) => format!("Invalid RTF file: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_paragraphs_and_skips_control_tables() {
        let rtf = br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil\fcharset0 Times New Roman;}}{\colortbl ;\red0\green0\blue0;}\viewkind4\uc1\pard\f0\fs20 Gold Mine\par Mine some gold.\par}";
        assert_eq!(rtf_to_plain_text(rtf), "Gold Mine\nMine some gold.\n");
    }

    #[test]
    fn decodes_hex_escapes_as_windows_1252() {
        let rtf = br"{\rtf1\ansi R\'e4uberh\'f6hle\par}";
        assert_eq!(rtf_to_plain_text(rtf), "Räuberhöhle\n");
    }

    #[test]
    fn skip_destination_star_groups_are_dropped() {
        let rtf = br"{\rtf1 kept{\*\generator Riched20;}\par}";
        assert_eq!(rtf_to_plain_text(rtf), "kept\n");
    }

    #[test]
    fn literal_braces_and_backslash_unescape() {
        let rtf = br"{\rtf1 a\{b\}c\\d}";
        assert_eq!(rtf_to_plain_text(rtf), "a{b}c\\d");
    }

    #[test]
    fn unclosed_block_reports_parser_detail() {
        assert_eq!(
            rtf_to_plain_text(br"{\rtf1 text"),
            "Invalid RTF file: Block not closed"
        );
    }

    #[test]
    fn excess_closing_brace_reports_parser_detail() {
        assert_eq!(
            rtf_to_plain_text(b"}"),
            "Invalid RTF file: Too many brackets closed"
        );
    }

    #[test]
    fn invalid_hex_escape_reports_parser_detail() {
        assert_eq!(
            rtf_to_plain_text(br"{\rtf1 \'g0}"),
            "Invalid RTF file: Invalid hex character"
        );
    }

    #[test]
    fn backslash_at_eof_reports_parser_detail() {
        assert_eq!(
            rtf_to_plain_text(b"\\"),
            "Invalid RTF file: Unexpected end of file"
        );
    }

    #[test]
    fn direct_rtf_parser_does_not_apply_native_nul_truncation() {
        assert_eq!(
            rtf_to_plain_text(b"{\\rtf1 Visible\\par}\0}"),
            "Invalid RTF file: Too many brackets closed"
        );
    }
}
