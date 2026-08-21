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

#[cfg(test)]
mod tests {
    use super::matches;

    /// `WildcardMatch` (`src/StdFile.cpp:337-367`, oracle `7d43b47`) decides
    /// every listing filter and every sort-list segment, and had no test at all.
    ///
    /// ```cpp
    /// while (*pWild || pLWild)
    ///     if (*pWild == '*')            { pLWild = ++pWild; pLPos = pPos; }
    ///     else if (!*pPos)              break;
    ///     else if (*pWild == '?' || tolower(*pWild) == tolower(*pPos)) { pWild++; pPos++; }
    ///     else if (pLPos)               { pWild = pLWild; pPos = ++pLPos; }
    ///     else                          return false;
    /// return !*pWild && !*pPos;
    /// ```
    ///
    /// The cases below are the ones where a plausible reimplementation drifts,
    /// each traced through the C++ above rather than guessed:
    ///
    /// * the final line demands **both** sides be exhausted, so a pattern that
    ///   still has literal characters left fails even after a `*` matched;
    /// * a trailing `*` still succeeds against an exhausted string, because the
    ///   `!*pPos` arm `break`s out to that same final check rather than failing;
    /// * `pLWild`/`pLPos` backtracking retries a `*` one character further along,
    ///   which is what makes a late literal findable;
    /// * matching is case-insensitive, via `tolower` on both sides.
    #[test]
    fn wildcard_matching_follows_the_cpp_exhaustion_and_backtracking_rules() {
        // Exact and case-insensitive, since `tolower` is applied to both sides.
        assert!(matches("Scenario.txt", "Scenario.txt"));
        assert!(matches("SCENARIO.TXT", "scenario.txt"));
        assert!(!matches("Scenario.txt", "Scenario.bin"));

        // `?` consumes exactly one character — never zero.
        assert!(matches("Sc?nario.txt", "Scenario.txt"));
        assert!(!matches("Sc?nario.txt", "Scnario.txt"));

        // `*` spans any run, including an empty one.
        assert!(matches("*.txt", "Scenario.txt"));
        assert!(matches("Scenario*", "Scenario.txt"));
        assert!(matches("Scenario*", "Scenario"));
        assert!(matches("*", ""));
        assert!(matches("", ""));

        // `return !*pWild && !*pPos` — the pattern must be exhausted too, so a
        // literal left after the `*` fails rather than being ignored.
        assert!(!matches("Scenario*.txt", "Scenario"));
        assert!(!matches("Sc", "Scenario"));
        assert!(!matches("", "Scenario"));

        // Backtracking: the `*` is retried one character further along until the
        // trailing literal lines up, which a greedy non-backtracking matcher
        // gets wrong on a repeated character.
        assert!(matches("*b", "abcb"));
        assert!(matches("*c*b", "acxcb"));
        assert!(!matches("*b", "abcx"));
        // The discriminating case: the retry must resume from the star's own
        // cursor (`pPos = ++pLPos`), not from wherever the failed attempt
        // stopped. Advancing the failed position instead skips the second 'a'
        // and never finds the "ab" tail.
        assert!(matches("*ab", "aab"));

        // A `*` before an exhausted string breaks straight to the final check,
        // so any number of trailing stars still matches.
        assert!(matches("Scenario**", "Scenario"));
    }
}
