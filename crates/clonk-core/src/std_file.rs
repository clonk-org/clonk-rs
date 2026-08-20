use std::fs;
use std::io;
use std::path::Path;

pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (mut pattern_index, mut text_index) = (0, 0);
    let (mut star_index, mut match_index) = (None, 0usize);
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    while text_index < text.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index].eq_ignore_ascii_case(&text[text_index]))
        {
            pattern_index += 1;
            text_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            match_index = text_index;
            pattern_index += 1;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            match_index += 1;
            text_index = match_index;
        } else {
            return false;
        }
    }

    pattern[pattern_index..].iter().all(|byte| *byte == b'*')
}

pub fn copy_file(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
    fail_if_exists: bool,
) -> io::Result<u64> {
    if fail_if_exists && to.as_ref().exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target exists",
        ));
    }
    fs::copy(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("*.txt", "readme.txt"));
        assert!(!wildcard_match("*.txt", "image.png"));
    }

    #[test]
    fn copy_file_honors_the_existing_target_guard() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();

        assert_eq!(
            copy_file(&source, &target, true).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        copy_file(&source, &target, false).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
    }
}
