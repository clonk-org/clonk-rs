//! `WildcardMatch` for the listing filter and the sort list.

/// `WildcardMatch`-style matching for the listing filter: `*` spans any run and
/// `?` one character.
pub fn matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some('?') => {
                p += 1;
                n += 1;
            }
            Some(character) if character.eq_ignore_ascii_case(&name[n]) => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(index) => {
                    p = index + 1;
                    resume += 1;
                    n = resume;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}
