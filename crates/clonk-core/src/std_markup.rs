#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkupTag {
    Italic,
    Color(u32),
}

pub struct Markup {
    tags: Vec<MarkupTag>,
}

impl Markup {
    pub fn new(_apply_color: bool) -> Self {
        Self { tags: Vec::new() }
    }

    pub fn read(&mut self, text: &mut &str, skip: bool) -> bool {
        if !text.starts_with('<') {
            return false;
        }
        let end = match text.find('>') {
            Some(idx) => idx,
            None => return false,
        };
        let tag_content = &text[1..end];
        let mut parts = tag_content.splitn(2, ' ');
        let name = parts.next().unwrap_or("");
        let params = parts.next().map(str::trim);

        if name.starts_with('/') {
            if params.is_some() {
                return false;
            }
            if !skip {
                match self.tags.pop() {
                    Some(last) if tag_name(&last) == &name[1..] => {}
                    _ => return false,
                }
            }
        } else if name == "i" {
            if params.is_some() {
                return false;
            }
            if !skip {
                self.tags.push(MarkupTag::Italic);
            }
        } else if name == "c" {
            let param = match params {
                Some(p) => p,
                None => return false,
            };
            if param.is_empty() || param.len() > 8 {
                return false;
            }
            let mut value: u32 = 0;
            for ch in param.chars() {
                let digit = match ch.to_digit(16) {
                    Some(d) => d,
                    None => return false,
                };
                value = (value << 4) | digit;
            }
            if param.len() <= 6 {
                value |= 0xff00_0000;
            }
            value = invert_rgba_alpha(value);
            if !skip {
                self.tags.push(MarkupTag::Color(value));
            }
        } else {
            return false;
        }
        *text = &text[end + 1..];
        true
    }

    pub fn skip_tags(&mut self, text: &mut &str) -> bool {
        while text.starts_with('<') {
            if !self.read(text, true) {
                break;
            }
        }
        text.is_empty()
    }

    pub fn to_markup(&self) -> String {
        self.tags
            .iter()
            .map(tag_to_markup)
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn to_close_markup(&self) -> String {
        self.tags
            .iter()
            .rev()
            .map(|tag| format!("</{}>", tag_name(tag)))
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn clean(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn tags(&self) -> &[MarkupTag] {
        &self.tags
    }

    pub fn strip_markup(text: &mut String) -> bool {
        let original = text.clone();
        let mut reader = text.as_str();
        let mut output = String::with_capacity(text.len());
        let mut markup = Markup::new(false);

        while !reader.is_empty() {
            let before = reader;
            markup.skip_tags(&mut reader);
            if before.len() != reader.len() {
                continue;
            }
            if reader.starts_with("{{") && !reader.starts_with("{{{") {
                if let Some(end) = reader[2..].find("}}") {
                    reader = &reader[2 + end + 2..];
                    continue;
                } else {
                    reader = &reader[2..];
                    continue;
                }
            }
            if reader.starts_with("}}") {
                reader = &reader[2..];
                continue;
            }
            let ch = reader.chars().next().unwrap();
            output.push(ch);
            reader = &reader[ch.len_utf8()..];
        }

        let changed = output != original;
        *text = output;
        changed
    }
}

fn tag_name(tag: &MarkupTag) -> &'static str {
    match tag {
        MarkupTag::Italic => "i",
        MarkupTag::Color(_) => "c",
    }
}

fn tag_to_markup(tag: &MarkupTag) -> String {
    match tag {
        MarkupTag::Italic => "<i>".into(),
        MarkupTag::Color(color) => format!("<c {:x}>", color),
    }
}

fn invert_rgba_alpha(color: u32) -> u32 {
    (color & 0x00ff_ffff) | ((255 - ((color >> 24) & 0xff)) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_close_markup() {
        let mut text = "<i><c ff0000>";
        let mut markup = Markup::new(true);
        assert!(markup.read(&mut text, false));
        assert!(markup.read(&mut text, false));
        assert_eq!(
            markup.tags(),
            &[MarkupTag::Italic, MarkupTag::Color(0x00ff_0000)]
        );
        assert_eq!(markup.to_markup(), "<i><c ff0000>");
        assert_eq!(markup.to_close_markup(), "</c></i>");
    }

    #[test]
    fn strip_markup_simple() {
        let mut text = "<i>Test</i>".to_string();
        assert!(Markup::strip_markup(&mut text));
        assert_eq!(text, "Test");
    }

    #[test]
    fn strip_markup_handles_unterminated_inline_tag() {
        let mut text = "{{broken".to_string();
        assert!(Markup::strip_markup(&mut text));
        assert_eq!(text, "broken");
    }
}
