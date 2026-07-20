use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug)]
pub(crate) struct SystemFontFace {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) face_index: u32,
}

pub(crate) trait SystemFontProvider: Send + Sync {
    fn resolve(&self, family: &str, weight: u32) -> Option<SystemFontFace>;
}

pub(crate) fn installed_system_fonts() -> &'static dyn SystemFontProvider {
    static PROVIDER: OnceLock<InstalledSystemFonts> = OnceLock::new();
    PROVIDER.get_or_init(InstalledSystemFonts::default)
}

#[derive(Default)]
struct InstalledSystemFonts {
    faces: OnceLock<Vec<IndexedFace>>,
    file_bytes: Mutex<HashMap<PathBuf, Arc<[u8]>>>,
}

#[derive(Debug)]
struct IndexedFace {
    path: PathBuf,
    face_index: u32,
    families: Vec<String>,
    weight: u16,
}

impl SystemFontProvider for InstalledSystemFonts {
    fn resolve(&self, family: &str, weight: u32) -> Option<SystemFontFace> {
        if family.is_empty() {
            return None;
        }

        let requested_family = family.to_lowercase();
        let requested_weight = weight.min(u16::MAX.into()) as u16;
        let faces = self.faces.get_or_init(scan_installed_fonts);
        let mut candidates = faces
            .iter()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|candidate| candidate.to_lowercase() == requested_family)
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|left, right| {
            left.weight
                .abs_diff(requested_weight)
                .cmp(&right.weight.abs_diff(requested_weight))
                .then_with(|| left.weight.cmp(&right.weight))
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.face_index.cmp(&right.face_index))
        });

        for face in candidates {
            if let Some(bytes) = self.bytes_for_path(&face.path) {
                return Some(SystemFontFace {
                    bytes,
                    face_index: face.face_index,
                });
            }
        }

        None
    }
}

impl InstalledSystemFonts {
    fn bytes_for_path(&self, path: &Path) -> Option<Arc<[u8]>> {
        if let Some(bytes) = self.file_bytes.lock().ok()?.get(path).cloned() {
            return Some(bytes);
        }

        let bytes = Arc::<[u8]>::from(fs::read(path).ok()?);
        let mut cache = self.file_bytes.lock().ok()?;
        Some(
            cache
                .entry(path.to_path_buf())
                .or_insert_with(|| bytes.clone())
                .clone(),
        )
    }
}

fn scan_installed_fonts() -> Vec<IndexedFace> {
    let mut font_files = Vec::new();
    let mut visited_directories = HashSet::new();
    for directory in standard_font_directories() {
        collect_font_files(&directory, &mut visited_directories, &mut font_files);
    }
    font_files.sort();
    font_files.dedup();

    let mut faces = Vec::new();
    for path in font_files {
        scan_font_file(path, &mut faces);
    }
    faces
}

fn collect_font_files(
    directory: &Path,
    visited_directories: &mut HashSet<PathBuf>,
    font_files: &mut Vec<PathBuf>,
) {
    let canonical_directory = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(_) => return,
    };
    if !visited_directories.insert(canonical_directory.clone()) {
        return;
    }

    let mut entries = match fs::read_dir(&canonical_directory) {
        Ok(entries) => entries.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return,
    };
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            collect_font_files(&path, visited_directories, font_files);
        } else if metadata.is_file() && is_supported_font_file(&path) {
            font_files.push(fs::canonicalize(&path).unwrap_or(path));
        }
    }
}

fn is_supported_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("ttf")
                || extension.eq_ignore_ascii_case("otf")
                || extension.eq_ignore_ascii_case("ttc")
                || extension.eq_ignore_ascii_case("otc")
        })
        .unwrap_or(false)
}

fn scan_font_file(path: PathBuf, faces: &mut Vec<IndexedFace>) {
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let face_count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);

    for face_index in 0..face_count {
        let face = match ttf_parser::Face::parse(&bytes, face_index) {
            Ok(face) => face,
            Err(_) => continue,
        };
        if face.style() != ttf_parser::Style::Normal
            || face.is_italic()
            || face.width() != ttf_parser::Width::Normal
        {
            continue;
        }

        let families = family_names(&face);
        if families.is_empty() {
            continue;
        }
        faces.push(IndexedFace {
            path: path.clone(),
            face_index,
            families,
            weight: face.weight().to_number(),
        });
    }
}

fn family_names(face: &ttf_parser::Face<'_>) -> Vec<String> {
    let mut families = Vec::new();
    for name in face.names() {
        if !matches!(
            name.name_id,
            ttf_parser::name_id::FAMILY
                | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                | ttf_parser::name_id::WWS_FAMILY
        ) {
            continue;
        }
        let Some(family) = name.to_string() else {
            continue;
        };
        if family.is_empty()
            || families
                .iter()
                .any(|known: &String| known.to_lowercase() == family.to_lowercase())
        {
            continue;
        }
        families.push(family);
    }
    families
}

fn standard_font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(windows) = env::var_os("WINDIR").or_else(|| env::var_os("SystemRoot")) {
            directories.push(PathBuf::from(windows).join("Fonts"));
        }
        if let Some(local_data) = env::var_os("LOCALAPPDATA") {
            directories.push(
                PathBuf::from(local_data)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        if let Some(roaming_data) = env::var_os("APPDATA") {
            directories.push(
                PathBuf::from(roaming_data)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        directories.extend([
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
            PathBuf::from("/Network/Library/Fonts"),
        ]);
        if let Some(home) = env::var_os("HOME") {
            directories.push(PathBuf::from(home).join("Library/Fonts"));
        }
        let mobile_assets = Path::new("/System/Library/AssetsV2");
        if let Ok(entries) = fs::read_dir(mobile_assets) {
            directories.extend(entries.filter_map(Result::ok).filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("com_apple_MobileAsset_Font"))
                    .then(|| entry.path())
            }));
        }
    }

    #[cfg(target_os = "android")]
    {
        directories.extend([
            PathBuf::from("/system/fonts"),
            PathBuf::from("/product/fonts"),
            PathBuf::from("/data/fonts"),
        ]);
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        directories.extend([
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
            PathBuf::from("/usr/X11R6/lib/X11/fonts"),
        ]);

        if let Some(home) = env::var_os("HOME") {
            let home = PathBuf::from(home);
            directories.push(home.join(".fonts"));
            if env::var_os("XDG_DATA_HOME").is_none() {
                directories.push(home.join(".local/share/fonts"));
            }
        }
        if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
            directories.push(PathBuf::from(data_home).join("fonts"));
        }
        if let Some(data_directories) = env::var_os("XDG_DATA_DIRS") {
            directories.extend(env::split_paths(&data_directories).map(|path| path.join("fonts")));
        }
    }

    let mut seen = HashSet::new();
    directories.retain(|directory| seen.insert(directory.clone()));
    directories
}
