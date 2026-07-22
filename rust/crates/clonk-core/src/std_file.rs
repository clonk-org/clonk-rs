use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::rngs::SmallRng;
use rand::{RngCore, SeedableRng};

#[cfg(target_os = "windows")]
const DIRECTORY_SEPARATOR: char = '\\';
#[cfg(not(target_os = "windows"))]
const DIRECTORY_SEPARATOR: char = '/';

#[cfg(target_os = "windows")]
const ALT_DIRECTORY_SEPARATOR: char = '/';
#[cfg(not(target_os = "windows"))]
const ALT_DIRECTORY_SEPARATOR: char = '\\';

pub fn get_working_directory() -> io::Result<PathBuf> {
    env::current_dir()
}

pub fn set_working_directory<P: AsRef<Path>>(path: P) -> io::Result<()> {
    env::set_current_dir(path)
}

pub fn get_filename(path: impl AsRef<Path>) -> Option<String> {
    path.as_ref()
        .file_name()
        .and_then(|os| os.to_str().map(ToString::to_string))
}

pub fn get_extension(path: impl AsRef<Path>) -> Option<String> {
    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str().map(ToString::to_string))
}

pub fn default_extension(path: &mut String, extension: &str) {
    if get_extension(&*path).is_none() && !extension.is_empty() {
        path.push('.');
        path.push_str(extension.trim_start_matches('.'));
    }
}

pub fn enforce_extension(path: &mut String, extension: &str) {
    remove_extension(path);
    default_extension(path, extension);
}

pub fn remove_extension(path: &mut String) {
    if let Some(pos) = path.rfind('.') {
        if path[pos..].contains(DIRECTORY_SEPARATOR)
            || path[pos..].contains(ALT_DIRECTORY_SEPARATOR)
        {
            return;
        }
        path.truncate(pos);
    }
}

pub fn append_backslash(path: &mut String) {
    if !path.ends_with(DIRECTORY_SEPARATOR) && !path.ends_with(ALT_DIRECTORY_SEPARATOR) {
        path.push(DIRECTORY_SEPARATOR);
    }
}

pub fn truncate_backslash(path: &mut String) {
    while path.ends_with(DIRECTORY_SEPARATOR) || path.ends_with(ALT_DIRECTORY_SEPARATOR) {
        path.pop();
    }
}

pub fn file_exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_file()
}

pub fn directory_exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_dir()
}

pub fn file_size(path: impl AsRef<Path>) -> io::Result<u64> {
    Ok(fs::metadata(path)?.len())
}

pub fn file_time(path: impl AsRef<Path>) -> io::Result<SystemTime> {
    fs::metadata(path)?.modified()
}

pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (mut p_idx, mut t_idx) = (0, 0);
    let (mut star_idx, mut match_idx) = (None, 0usize);
    let pat = pattern.as_bytes();
    let txt = text.as_bytes();

    while t_idx < txt.len() {
        if p_idx < pat.len() && (pat[p_idx] == b'?' || pat[p_idx].eq_ignore_ascii_case(&txt[t_idx]))
        {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < pat.len() && pat[p_idx] == b'*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
        } else if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < pat.len() && pat[p_idx] == b'*' {
        p_idx += 1;
    }

    p_idx == pat.len()
}

pub fn wildcard_list_match(list: &str, text: &str) -> bool {
    list.split('|')
        .any(|pattern| wildcard_match(pattern.trim(), text))
}

pub fn erase_file(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_file(path)
}

pub fn rename_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    fs::rename(from, to)
}

pub fn make_directory(path: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(path)
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

pub fn erase_directory(path: impl AsRef<Path>) -> io::Result<()> {
    if path.as_ref().exists() {
        fs::remove_dir_all(path)
    } else {
        Ok(())
    }
}

pub fn copy_directory(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    reset_attributes: bool,
) -> io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    make_directory(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let mut target = dst.to_path_buf();
        target.push(entry.file_name());
        if entry.path() == dst {
            continue;
        }
        if file_type.is_dir() {
            copy_directory(entry.path(), &target, reset_attributes)?;
        } else {
            fs::copy(entry.path(), &target)?;
            if reset_attributes {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&target)?.permissions();
                    perms.set_mode(0o644);
                    fs::set_permissions(&target, perms)?;
                }
            }
        }
    }
    Ok(())
}

pub fn move_item(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    fs::rename(from, to)
}

pub fn item_identical(one: impl AsRef<Path>, two: impl AsRef<Path>) -> io::Result<bool> {
    let meta_one = fs::metadata(one)?;
    let meta_two = fs::metadata(two)?;
    Ok(meta_one.len() == meta_two.len() && meta_one.modified()? == meta_two.modified()?)
}

pub fn make_temp_filename(prefix: &str) -> io::Result<PathBuf> {
    let mut temp = env::temp_dir();
    let unique = format!(
        "{}-{:x}-{:x}",
        prefix,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SmallRng::from_entropy().next_u64()
    );
    temp.push(unique);
    Ok(temp)
}

pub fn read_file_line(file: &mut BufReader<File>, buffer: &mut String) -> io::Result<bool> {
    buffer.clear();
    let bytes = file.read_line(buffer)?;
    if bytes == 0 {
        return Ok(false);
    }
    if buffer.ends_with('\n') {
        buffer.pop();
        if buffer.ends_with('\r') {
            buffer.pop();
        }
    }
    Ok(true)
}

pub fn advance_file_line(file: &mut BufReader<File>) -> io::Result<()> {
    let mut dummy = String::new();
    let _ = read_file_line(file, &mut dummy)?;
    Ok(())
}

pub fn get_parent_path(path: impl AsRef<Path>) -> Option<PathBuf> {
    path.as_ref().parent().map(|p| p.to_path_buf())
}

pub fn get_relative_path(path: impl AsRef<Path>, relative_to: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    let relative_to = relative_to.as_ref();
    let relative_to_comps = relative_to.components().collect::<Vec<_>>();
    let components = path.components().collect::<Vec<_>>();
    if path == relative_to {
        return Some(PathBuf::new());
    }
    let mut idx = 0;
    while idx < relative_to_comps.len()
        && idx < components.len()
        && relative_to_comps[idx] == components[idx]
    {
        idx += 1;
    }
    if idx == 0 {
        return None;
    }
    let mut relative = PathBuf::new();
    for _ in idx..relative_to_comps.len() {
        relative.push("..");
    }
    for comp in &components[idx..] {
        relative.push(comp.as_os_str());
    }
    Some(relative)
}

pub fn is_global_path(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if path.is_absolute() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(first) = path.components().next() {
            if let std::path::Component::Prefix(prefix) = first {
                return matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                );
            }
        }
    }
    false
}

pub struct DirectoryIterator {
    entries: Vec<PathBuf>,
    index: usize,
}

impl DirectoryIterator {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut entries: Vec<_> = fs::read_dir(path)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        Ok(Self { entries, index: 0 })
    }

    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            index: 0,
        }
    }
}

impl Iterator for DirectoryIterator {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.entries.len() {
            None
        } else {
            let item = self.entries[self.index].clone();
            self.index += 1;
            Some(item)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn filename_helpers() {
        let mut path = String::from("dir/example.txt");
        assert_eq!(get_filename(&path), Some("example.txt".into()));
        assert_eq!(get_extension(&path), Some("txt".into()));
        remove_extension(&mut path);
        assert_eq!(path, "dir/example");
        default_extension(&mut path, "dat");
        assert_eq!(path, "dir/example.dat");
        enforce_extension(&mut path, "cfg");
        assert_eq!(path, "dir/example.cfg");
        append_backslash(&mut path);
        assert!(path.ends_with(std::path::MAIN_SEPARATOR));
        truncate_backslash(&mut path);
        assert!(!path.ends_with(std::path::MAIN_SEPARATOR));
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("*.txt", "readme.txt"));
        assert!(!wildcard_match("*.txt", "image.png"));
        assert!(wildcard_list_match("*.png|*.jpg", "cover.jpg"));
    }

    #[test]
    fn file_system_roundtrip() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("demo.txt");
        fs::write(&file_path, b"hello").unwrap();

        assert!(file_exists(&file_path));
        assert!(!directory_exists(&file_path));
        assert_eq!(file_size(&file_path).unwrap(), 5);

        let copy_path = dir.path().join("copy.txt");
        copy_file(&file_path, &copy_path, true).unwrap();
        assert!(file_exists(&copy_path));

        rename_file(&copy_path, dir.path().join("renamed.txt")).unwrap();

        let nested = dir.path().join("nested");
        copy_directory(dir.path(), &nested, false).unwrap();
        assert!(directory_exists(&nested));

        erase_directory(&nested).unwrap();
        assert!(!directory_exists(&nested));
    }

    #[test]
    fn directory_iterator_collects() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();
        fs::write(dir.path().join("b.txt"), b"b").unwrap();

        let iter = DirectoryIterator::new(dir.path()).unwrap();
        let mut collected: Vec<_> = iter
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        collected.sort();
        assert_eq!(collected, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn read_lines_helper() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        fs::write(&path, b"line1\nline2\r\n").unwrap();
        let file = File::open(&path).unwrap();
        let mut reader = BufReader::new(file);
        let mut buffer = String::new();
        assert!(read_file_line(&mut reader, &mut buffer).unwrap());
        assert_eq!(buffer, "line1");
        assert!(read_file_line(&mut reader, &mut buffer).unwrap());
        assert_eq!(buffer, "line2");
        assert!(!read_file_line(&mut reader, &mut buffer).unwrap());
    }
}
