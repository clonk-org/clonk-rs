use crate::{
    bitmap::IndexedBitmap, decode_legacy_script_text, graphics::blacken_fully_transparent_rgba,
    language::component_language_string, ComponentGroups, GraphicsImage, Group, GroupEntry,
    GroupError, LoadedComponent, ResourceLoadDiagnostic,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

const C4D_MAX_VERTEX: usize = 30;
const C4D_SORT_LIMIT: i32 = (1 << 5) - 1;
const C4D_CREW_MEMBER: i32 = 1 << 18;

/// C4AllowPictureStack bits (src/C4Constants.h:301-309).
pub const APS_COLOR: i32 = 1 << 0;
pub const APS_GRAPHICS: i32 = 1 << 1;
pub const APS_NAME: i32 = 1 << 2;
pub const APS_OVERLAY: i32 = 1 << 3;
const C4M_SOLID: i32 = 50;

/// Files required to construct an engine definition from a classic C4 definition folder.
#[derive(Debug, Clone)]
pub struct Definition {
    pub core: DefCore,
    pub script: DefinitionScript,
    pub action_map: Option<ActionMap>,
    pub picture_image: Option<GraphicsImage>,
    /// ColorByOwner mask cropped to `picture_image`'s `Picture` facet.
    /// C++ obtains both through `Graphics.GetBitmap(color)` in
    /// `C4Def::Picture2Facet` (src/C4Def.cpp:1374-1378).
    pub picture_color_by_owner_mask: Option<ColorByOwnerMask>,
    pub graphics_image: Option<GraphicsImage>,
    pub color_by_owner_mask: Option<ColorByOwnerMask>,
    pub additional_graphics: HashMap<String, DefinitionGraphicsVariant>,
    /// First `Portrait*.*` def portrait (C4CFN_Portraits,
    /// src/C4Components.h:88). C++ assigns fresh crew a *non-synced* random
    /// portrait from the def set (`C4ObjectInfo::SetRandomPortrait`,
    /// src/C4ObjectInfo.cpp:398-425); the Rust HUD deterministically shows
    /// the first.
    pub portrait_image: Option<GraphicsImage>,
    /// ColorByOwner-aware form of `portrait_image`. Kept separately so
    /// existing portrait consumers can migrate without changing while the
    /// C++ text-image path gains its required owner-color surface.
    pub portrait_graphics_image: Option<GraphicsImage>,
    /// ColorByOwner mask paired with `portrait_graphics_image`. C++ maps
    /// Portrait1.png to Overlay1.png while loading definition graphics
    /// (C4DefGraphics.cpp:166-205).
    pub portrait_color_by_owner_mask: Option<ColorByOwnerMask>,
    /// Every `Portrait*.*` graphics entry in group order. `name` is the
    /// suffix after `Portrait` (for example `1` or `IndianChief`).
    pub portrait_graphics: Vec<DefinitionGraphicsVariant>,
    /// The def's own rank symbol strip (`C4Def::pRankSymbols` from
    /// Rank.png, src/C4Def.cpp:684-691).
    pub rank_symbols_image: Option<GraphicsImage>,
    /// Number of base rank cells in `rank_symbols_image`, after subtracting
    /// the extension cells named by leading-`*` entries in the selected
    /// localized `Rank*.txt` (`C4Def::iNumRankSymbols`,
    /// src/C4Def.cpp:694-706). `None` means no valid custom rank strip.
    pub rank_symbol_count: Option<u32>,
    /// Custom rank names from the first localized
    /// `Rank{language}.txt|Rank.txt` component. `C4RankSystem` exposes base
    /// names first, followed by every leading-`*` extension format applied to
    /// every base name in order. Extension formatting remains lazy so an
    /// invalid format fails only when its rank is requested, as in native
    /// `GetRankName` (src/C4RankSystem.cpp:96-180,184-211).
    /// `None` means that no component was present or that C++ would reject it
    /// for containing no ordinary rank name.
    pub rank_names: Option<RankNameTable>,
    /// Experience curve base from the selected definition rank component's
    /// exact, case-sensitive `Base=` setting. Valid custom rank components
    /// default to 1000, matching `C4RankSystem`; invalid or absent components
    /// have no definition-local base.
    pub rank_base: Option<i32>,
    /// Raw selected `ClonkNames{language}.txt|ClonkNames.txt` contents.
    /// Loading remains gated by a local `ClonkNames*.txt` entry even when a
    /// language-pack component wins the candidate search (C4Def.cpp:641-657).
    pub clonk_names: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ColorByOwnerMask {
    pub width: u32,
    pub height: u32,
    /// Auto-generated owner-color surfaces use one grayscale byte per pixel.
    /// Explicit Overlay*.png surfaces retain four RGBA bytes per pixel so
    /// colored texels and partial alpha survive through the renderer.
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DefinitionGraphicsVariant {
    pub name: String,
    pub image: GraphicsImage,
    pub color_by_owner_mask: Option<ColorByOwnerMask>,
}

impl Definition {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        Self::load_with_languages(group, &crate::scenario::DEFAULT_LANGUAGE_SEQUENCE)
    }

    pub fn load_with_languages<S: AsRef<str>>(
        group: &Group,
        languages: &[S],
    ) -> Result<Self, DefinitionError> {
        Self::load_with_languages_and_components(group, languages, &ComponentGroups::local(group))
    }

    /// Loads the remaining definition resources after the caller has already
    /// parsed `DefCore.txt` for ID filtering.
    pub fn load_with_core_and_languages<S: AsRef<str>>(
        group: &Group,
        core: DefCore,
        languages: &[S],
    ) -> Result<Self, DefinitionError> {
        Self::load_with_core_and_languages_and_components(
            group,
            core,
            languages,
            &ComponentGroups::local(group),
        )
    }

    pub fn load_with_languages_and_components<S: AsRef<str>>(
        group: &Group,
        languages: &[S],
        components: &ComponentGroups,
    ) -> Result<Self, DefinitionError> {
        let core = DefCore::load(group)?;
        Self::load_with_core_and_languages_and_components(group, core, languages, components)
    }

    pub fn load_with_core_and_languages_and_components<S: AsRef<str>>(
        group: &Group,
        mut core: DefCore,
        languages: &[S],
        components: &ComponentGroups,
    ) -> Result<Self, DefinitionError> {
        if let Some(name) = load_definition_name(components, languages)? {
            core.name = Some(name);
        }

        let mut script = load_scripts(group, languages)?;

        let action_map = load_optional_entry_string(group, "ActMap.txt")?
            .map(|bytes| parse_act_map(&bytes))
            .transpose()?;
        script.definition_description = load_definition_description(components, languages)?;
        let clonk_names = if has_local_clonk_name_file(group)? {
            load_definition_clonk_names(components, languages)?
        } else {
            None
        };

        let (graphics_image, color_by_owner_mask, additional_graphics) =
            load_definition_graphics(group, core.color_by_owner)?;
        let picture_image = crop_definition_picture(&core, graphics_image.as_ref());
        let picture_color_by_owner_mask = crop_definition_picture_mask(
            &core,
            picture_image.as_ref(),
            color_by_owner_mask.as_ref(),
        );
        let portrait_image = load_plain_image(group, "Portrait1.png");
        let (portrait_graphics_image, portrait_color_by_owner_mask) =
            load_graphics_entry(group, Path::new("Portrait1.png"), core.color_by_owner)?
                .map(|(image, mask)| (Some(image), mask))
                .unwrap_or((None, None));
        let portrait_graphics = load_portrait_graphics(group, core.color_by_owner)?;
        // C4Def chooses the PNG branch by entry presence; a corrupt PNG does
        // not fall through to a valid legacy BMP (C4Def.cpp:684-691).
        let rank_symbols_image = if group.exists("Rank.png") {
            load_plain_image(group, "Rank.png")
        } else {
            load_plain_image(group, "Rank.bmp")
        }
        .filter(|image| image.height() > 0 && image.width() / image.height() > 0);
        let rank_name_table = if has_local_rank_name_file(group)? {
            load_rank_name_table(components, languages)?
        } else {
            None
        };
        let rank_extension_count = rank_name_table
            .as_ref()
            .map_or(0, |table| table.extension_count);
        let rank_symbol_count = rank_symbols_image.as_ref().and_then(|image| {
            let phase_count = image.width() / image.height().max(1);
            (phase_count > 0).then(|| phase_count.saturating_sub(rank_extension_count).max(1))
        });
        let (rank_names, rank_base) = rank_name_table
            .map(|table| (Some(table.names), Some(table.base)))
            .unwrap_or((None, None));

        Ok(Self {
            core,
            script,
            action_map,
            picture_image,
            picture_color_by_owner_mask,
            graphics_image,
            color_by_owner_mask,
            additional_graphics,
            portrait_image,
            portrait_graphics_image,
            portrait_color_by_owner_mask,
            portrait_graphics,
            rank_symbols_image,
            rank_symbol_count,
            rank_names,
            rank_base,
            clonk_names,
        })
    }

    /// Trimmed localized definition description exposed by
    /// `C4Def::GetDesc` (C4Def.cpp:713-717).
    pub fn description(&self) -> Option<&str> {
        self.script.definition_description.as_deref()
    }
}

/// Selects the first `C4CFN_DefDesc = "Desc{}.txt"` component admitted by
/// the exact language sequence. Unlike Names/Rank, this pattern has no plain
/// fallback segment; only an explicitly empty language code selects Desc.txt.
fn load_definition_description<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<String>, DefinitionError> {
    let mut selected = None;
    if languages.is_empty() {
        selected = components.read("Desc.txt")?;
    } else {
        for language in languages {
            let code = component_language_code(language.as_ref());
            let candidate = format!("Desc{code}.txt");
            if let Some(component) = components.read(candidate)? {
                selected = Some(component);
                break;
            }
        }
    }
    let Some(component) = selected else {
        return Ok(None);
    };

    let description = decode_legacy_script_text(&component.bytes)
        .trim_matches(|character: char| character.is_ascii_whitespace())
        .to_string();
    Ok((!description.is_empty()).then_some(description))
}

/// C4ComponentHost inserts at most two native bytes from each comma-separated
/// language segment (`SCopySegment(..., 2)`, C4ComponentHost.cpp:70-79).
fn component_language_code(language: &str) -> String {
    let code = clonk_script::c4_string_bytes(language);
    let visible = code
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(code.len());
    clonk_script::c4_string_from_bytes(&code[..visible.min(2)])
}

/// C4Def gates ClonkNames loading on a local `ClonkNames*.txt` wildcard
/// match before `LoadEx` may cross-load the selected component from a pack.
fn has_local_clonk_name_file(group: &Group) -> Result<bool, DefinitionError> {
    const PREFIX: &[u8] = b"ClonkNames";
    const SUFFIX: &[u8] = b".txt";

    Ok(group.entries()?.into_iter().any(|entry| {
        let name = entry.name_bytes;
        name.len() >= PREFIX.len() + SUFFIX.len()
            && name[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
            && name[name.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
    }))
}

fn load_definition_clonk_names<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<String>, DefinitionError> {
    Ok(
        first_localized_component(components, "ClonkNames", languages)?.map(|component| {
            let visible = component
                .bytes
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_default();
            decode_legacy_script_text(visible)
        }),
    )
}

/// C4Def performs this local wildcard probe before RankSystem::LoadEx. A
/// language pack may win candidate selection only after any local Rank*.txt
/// marker has enabled rank loading at all.
fn has_local_rank_name_file(group: &Group) -> Result<bool, DefinitionError> {
    Ok(group.entries()?.into_iter().any(|entry| {
        let name = entry.name_bytes;
        name.len() >= b"Rank.txt".len()
            && name[..4].eq_ignore_ascii_case(b"Rank")
            && name[name.len() - 4..].eq_ignore_ascii_case(b".txt")
    }))
}

/// `C4Def::Load`'s localized `C4CFN_DefNames = "Names{}.txt|Names.txt"`:
/// load the first filename admitted by the language sequence, then select
/// the first matching `XX:` line from that one component
/// (`C4Def.cpp:635-639`; `C4ComponentHost.cpp:55-94,238-260`).
fn load_definition_name<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<String>, DefinitionError> {
    let Some(component) = first_localized_component(components, "Names", languages)? else {
        return Ok(None);
    };
    let text = decode_legacy_script_text(&component.bytes);
    let localized_name = |code: &str| component_language_string(&text, code).map(str::to_string);
    if languages.is_empty() {
        Ok(localized_name(""))
    } else {
        Ok(languages
            .iter()
            .find_map(|language| localized_name(&component_language_code(language.as_ref()))))
    }
}

/// Selects `Stem{language}.txt|Stem.txt` with the same filename-first,
/// language-sequence and group-priority order as `C4ComponentHost::LoadEx`
/// (src/C4ComponentHost.cpp:65-153).
fn first_localized_component<S: AsRef<str>>(
    components: &ComponentGroups,
    stem: &str,
    languages: &[S],
) -> Result<Option<LoadedComponent>, DefinitionError> {
    for candidate in languages
        .iter()
        .map(|language| format!("{stem}{}.txt", component_language_code(language.as_ref())))
        .chain(std::iter::once_with(|| format!("{stem}.txt")))
    {
        if let Some(component) = components.read(candidate)? {
            return Ok(Some(component));
        }
    }
    Ok(None)
}

/// C++ `C4Group::LoadEntryString` reports both a missing entry and a present
/// zero-byte entry as not loaded. Keep generic byte reads unchanged because
/// the adjacent `LoadEntry` API accepts empty binary payloads.
fn load_optional_entry_string<P: AsRef<Path>>(
    group: &Group,
    relative: P,
) -> Result<Option<Vec<u8>>, DefinitionError> {
    match group.load_entry_string(relative) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GroupError::EntryNotFound(_) | GroupError::EmptyEntry(_)) => Ok(None),
        Err(GroupError::Io(ref err)) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DefinitionError::Resources(error)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedRankNameTable {
    names: RankNameTable,
    extension_count: u32,
    base: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RankBaseName {
    bytes: Vec<u8>,
    decoded: String,
}

/// A definition-local `C4RankSystem` name table.
///
/// Native retains extension format strings and invokes `fmt::sprintf` from
/// `GetRankName`. Keeping the same split here is observable for malformed
/// formats: loading the definition and requesting any base rank still work;
/// requesting an affected extended rank raises an uncaught Rust panic at the
/// corresponding native uncaught `fmt::format_error` boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankNameTable {
    inner: Arc<RankNameTableData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("rank extension format `{format}` could not be applied: {reason}")]
pub struct RankExtensionFormatError {
    pub format: String,
    pub reason: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
struct RankNameTableData {
    ordinary_names: Vec<RankBaseName>,
    extensions: Vec<Vec<u8>>,
}

impl RankNameTable {
    /// Construct a table whose names are already resolved. Engine-side test
    /// and embedding APIs use this for custom tables without extensions.
    pub fn from_resolved_names(names: Vec<String>) -> Self {
        Self {
            inner: Arc::new(RankNameTableData {
                ordinary_names: names
                    .into_iter()
                    .map(|decoded| RankBaseName {
                        bytes: decoded.as_bytes().to_vec(),
                        decoded,
                    })
                    .collect(),
                extensions: Vec::new(),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner
            .ordinary_names
            .len()
            .saturating_mul(self.inner.extensions.len().saturating_add(1))
    }

    pub fn is_empty(&self) -> bool {
        self.inner.ordinary_names.is_empty()
    }

    /// Resolve one rank exactly when it is requested, optionally clamping an
    /// over-range rank to the final table entry like native's
    /// `fReturnLastIfOver` path.
    pub fn try_rank_name(
        &self,
        rank: usize,
        return_last_if_over: bool,
    ) -> Result<Option<Cow<'_, str>>, RankExtensionFormatError> {
        let table_len = self.len();
        if table_len == 0 {
            return Ok(None);
        }
        let rank = if rank < table_len {
            rank
        } else if return_last_if_over {
            table_len - 1
        } else {
            return Ok(None);
        };
        let ordinary_count = self.inner.ordinary_names.len();
        if rank < ordinary_count {
            return Ok(Some(Cow::Borrowed(
                &self.inner.ordinary_names[rank].decoded,
            )));
        }
        let extended_rank = rank - ordinary_count;
        let extension = self
            .inner
            .extensions
            .get(extended_rank / ordinary_count)
            .expect("bounded rank references a defined extension");
        let ordinary = &self.inner.ordinary_names[extended_rank % ordinary_count];
        let formatted = format_rank_extension(extension, &ordinary.bytes).map_err(|reason| {
            RankExtensionFormatError {
                format: decode_legacy_script_text(extension),
                reason,
            }
        })?;
        Ok(Some(Cow::Owned(decode_legacy_script_text(&formatted))))
    }

    /// Resolve a rank through the normal non-fallback `GetRankName` path.
    /// Invalid formats deliberately panic here: native lets the corresponding
    /// `fmt::format_error` escape `GetRankName` uncaught.
    pub fn get(&self, rank: usize) -> Option<Cow<'_, str>> {
        self.try_rank_name(rank, false)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn get_or_last(&self, rank: usize) -> Option<Cow<'_, str>> {
        self.try_rank_name(rank, true)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn resolved_names(&self) -> Vec<String> {
        (0..self.len())
            .map(|rank| {
                self.get(rank)
                    .expect("rank table length only covers defined ranks")
                    .into_owned()
            })
            .collect()
    }
}

/// Loads the selected definition rank component in the order exposed by
/// `C4RankSystem::GetRankName`: ordinary names first, then each leading-`*`
/// extension across all ordinary names. Extension application stays deferred
/// to lookup. Comments and settings are retained by neither list, and a
/// component without an ordinary name is rejected (src/C4RankSystem.cpp:96-211).
fn load_rank_name_table<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<LoadedRankNameTable>, DefinitionError> {
    let Some(component) = first_localized_component(components, "Rank", languages)? else {
        return Ok(None);
    };
    let mut ordinary_names = Vec::new();
    let mut extensions = Vec::new();
    let mut base = 1000;
    // The C++ loop only processes lines when it encounters CR or LF within
    // the component data; its appended trailing NUL lies outside that loop.
    // Consequently an unterminated final line is intentionally ignored.
    // Embedded NUL bytes are terminators too because C++ tests `!*pPos`.
    for terminated_line in component
        .bytes
        .split_inclusive(|byte| matches!(*byte, 0 | b'\r' | b'\n'))
    {
        let Some((terminator, line)) = terminated_line.split_last() else {
            continue;
        };
        if !matches!(*terminator, 0 | b'\r' | b'\n') || line.is_empty() {
            continue;
        }
        if let Some(extension) = line.strip_prefix(b"*") {
            extensions.push(extension.to_vec());
        } else if let Some(parsed_base) = parse_rank_base(line) {
            base = parsed_base;
        } else if !line.starts_with(b"#") && !line.contains(&b'=') {
            ordinary_names.push(line.to_vec());
        }
    }
    if ordinary_names.is_empty() {
        return Ok(None);
    }

    let extension_count = u32::try_from(extensions.len()).unwrap_or(u32::MAX);
    let names = RankNameTable {
        inner: Arc::new(RankNameTableData {
            ordinary_names: ordinary_names
                .into_iter()
                .map(|bytes| RankBaseName {
                    decoded: decode_legacy_script_text(&bytes),
                    bytes,
                })
                .collect(),
            extensions,
        }),
    };
    Ok(Some(LoadedRankNameTable {
        names,
        extension_count,
        base: if base == 0 { 1000 } else { base },
    }))
}

/// Parse the `%d` prefix accepted by C++ for an exact `Base=` rank setting.
/// Leading ASCII whitespace and a sign are accepted; trailing bytes are
/// ignored. A malformed value leaves the previously parsed base unchanged.
fn parse_rank_base(line: &[u8]) -> Option<i32> {
    let value = line.strip_prefix(b"Base=")?;
    let value = &value[value
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count()..];
    let digits_start = usize::from(value.starts_with(b"+") || value.starts_with(b"-"));
    let digit_count = value[digits_start..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .count();
    if digit_count == 0 {
        return None;
    }
    std::str::from_utf8(&value[..digits_start + digit_count])
        .ok()?
        .parse()
        .ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RankPrintfArgumentMode {
    Unset,
    Automatic,
    Positional,
}

fn use_rank_printf_argument(
    mode: &mut RankPrintfArgumentMode,
    automatic_arguments: &mut usize,
    positional: Option<usize>,
) -> Result<(), &'static str> {
    if let Some(index) = positional {
        if *mode == RankPrintfArgumentMode::Automatic {
            return Err("cannot switch from automatic to manual argument indexing");
        }
        *mode = RankPrintfArgumentMode::Positional;
        return (index == 1).then_some(()).ok_or("argument not found");
    }
    if *mode == RankPrintfArgumentMode::Positional {
        return Err("cannot switch from manual to automatic argument indexing");
    }
    *mode = RankPrintfArgumentMode::Automatic;
    if *automatic_arguments != 0 {
        return Err("argument not found");
    }
    *automatic_arguments += 1;
    Ok(())
}

fn parse_rank_printf_number(format: &[u8], cursor: &mut usize) -> (usize, bool) {
    let start = *cursor;
    let mut value = 0u64;
    while let Some(digit) = format
        .get(*cursor)
        .filter(|byte| byte.is_ascii_digit())
        .map(|byte| u64::from(*byte - b'0'))
    {
        value = value.wrapping_mul(10).wrapping_add(digit);
        *cursor += 1;
    }
    // fmt 11.2 accepts at most nine decimal digits unconditionally, or ten
    // whose value fits INT_MAX. More digits overflow even when they are only
    // leading zeroes (base.h::parse_nonnegative_int).
    let digit_count = *cursor - start;
    let too_big = digit_count > 10 || (digit_count == 10 && value > i32::MAX as u64);
    (value as usize, too_big)
}

fn fmt_code_point_width(character: char) -> usize {
    let code = u32::from(character);
    1 + usize::from(
        code >= 0x1100
            && (code <= 0x115f
                || code == 0x2329
                || code == 0x232a
                || (0x2e80..=0xa4cf).contains(&code) && code != 0x303f
                || (0xac00..=0xd7a3).contains(&code)
                || (0xf900..=0xfaff).contains(&code)
                || (0xfe10..=0xfe19).contains(&code)
                || (0xfe30..=0xfe6f).contains(&code)
                || (0xff00..=0xff60).contains(&code)
                || (0xffe0..=0xffe6).contains(&code)
                || (0x20000..=0x2fffd).contains(&code)
                || (0x30000..=0x3fffd).contains(&code)
                || (0x1f300..=0x1f64f).contains(&code)
                || (0x1f900..=0x1f9ff).contains(&code)),
    )
}

fn fmt_utf8_display_width(bytes: &[u8]) -> usize {
    let mut width = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match std::str::from_utf8(&bytes[cursor..]) {
            Ok(text) => {
                width = width.saturating_add(text.chars().map(fmt_code_point_width).sum());
                break;
            }
            Err(error) => {
                let valid_end = cursor + error.valid_up_to();
                let valid = std::str::from_utf8(&bytes[cursor..valid_end])
                    .expect("from_utf8 valid prefix is UTF-8");
                width = width.saturating_add(valid.chars().map(fmt_code_point_width).sum());
                cursor = valid_end;
                if cursor < bytes.len() {
                    // fmt advances one byte and counts one column for every
                    // malformed UTF-8 code unit.
                    width = width.saturating_add(1);
                    cursor += 1;
                }
            }
        }
    }
    width
}

/// Apply the static one-`char *` surface of fmt 11.2 `sprintf` when the
/// corresponding extended rank is requested.
fn format_rank_extension(format: &[u8], base_name: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut output = Vec::with_capacity(format.len().saturating_add(base_name.len()));
    let mut cursor = 0usize;
    let mut argument_mode = RankPrintfArgumentMode::Unset;
    let mut automatic_arguments = 0usize;
    while cursor < format.len() {
        if format[cursor] != b'%' {
            output.push(format[cursor]);
            cursor += 1;
            continue;
        }
        cursor += 1;
        if format.get(cursor) == Some(&b'%') {
            output.push(b'%');
            cursor += 1;
            continue;
        }

        let mut positional = None;
        let mut left_aligned = false;
        let mut width = 0usize;
        let mut parse_flags_and_width = true;
        if format.get(cursor).is_some_and(u8::is_ascii_digit) {
            let (number, too_big) = parse_rank_printf_number(format, &mut cursor);
            if format.get(cursor) == Some(&b'$') {
                positional = Some(if too_big { usize::MAX } else { number });
                cursor += 1;
            } else if number != 0 || too_big {
                if too_big {
                    return Err("number is too big");
                }
                width = number;
                parse_flags_and_width = false;
            }
        }
        if parse_flags_and_width {
            while let Some(flag) = format.get(cursor) {
                match flag {
                    b'-' => left_aligned = true,
                    b'+' | b'0' | b' ' | b'#' => {}
                    _ => break,
                }
                cursor += 1;
            }
            if format.get(cursor).is_some_and(u8::is_ascii_digit) {
                let (number, too_big) = parse_rank_printf_number(format, &mut cursor);
                if too_big {
                    return Err("number is too big");
                }
                width = number;
            } else if format.get(cursor) == Some(&b'*') {
                use_rank_printf_argument(&mut argument_mode, &mut automatic_arguments, None)?;
                return Err("width is not integer");
            }
        }

        // fmt rejects argument zero after parsing the header (including a
        // dynamic width) but before parsing precision. Preserve that error
        // precedence for formats such as `%0$.*s`.
        if positional == Some(0) {
            return Err("argument not found");
        }

        let mut precision = None;
        if format.get(cursor) == Some(&b'.') {
            cursor += 1;
            if format.get(cursor).is_some_and(u8::is_ascii_digit) {
                let (number, too_big) = parse_rank_printf_number(format, &mut cursor);
                // fmt 11.2 passes zero as parse_nonnegative_int's overflow
                // sentinel for printf precision.
                precision = Some(if too_big { 0 } else { number });
            } else if format.get(cursor) == Some(&b'*') {
                use_rank_printf_argument(&mut argument_mode, &mut automatic_arguments, None)?;
                return Err("precision is not integer");
            } else {
                precision = Some(0);
            }
        }

        use_rank_printf_argument(&mut argument_mode, &mut automatic_arguments, positional)?;

        let mut had_length = false;
        match format.get(cursor).copied() {
            Some(b'h' | b'l') => {
                had_length = true;
                let length = format[cursor];
                cursor += 1;
                if format.get(cursor) == Some(&length) {
                    cursor += 1;
                }
            }
            Some(b'j' | b'z' | b't' | b'L') => {
                had_length = true;
                cursor += 1;
            }
            _ => {}
        }
        let Some(conversion) = format.get(cursor).copied() else {
            return Err(if had_length {
                "invalid format string"
            } else {
                "invalid format specifier"
            });
        };
        cursor += 1;

        let value = match conversion {
            b's' => &base_name[..precision.unwrap_or(base_name.len()).min(base_name.len())],
            b'p' if precision.is_none() => {
                let pointer = format!("0x{:x}", base_name.as_ptr() as usize).into_bytes();
                let display_width = pointer.len();
                let padding = width.saturating_sub(display_width);
                output
                    .try_reserve(pointer.len().saturating_add(padding))
                    .map_err(|_| "formatted rank is too large")?;
                if left_aligned {
                    output.extend_from_slice(&pointer);
                    output.resize(output.len() + padding, b' ');
                } else {
                    output.resize(output.len() + padding, b' ');
                    output.extend_from_slice(&pointer);
                }
                continue;
            }
            _ => return Err("invalid format specifier"),
        };
        let padding = width.saturating_sub(fmt_utf8_display_width(value));
        output
            .try_reserve(value.len().saturating_add(padding))
            .map_err(|_| "formatted rank is too large")?;
        if left_aligned {
            output.extend_from_slice(value);
            output.resize(output.len() + padding, b' ');
        } else {
            output.resize(output.len() + padding, b' ');
            output.extend_from_slice(value);
        }
    }
    Ok(output)
}

/// Decodes a single named image from the def group, `None` when absent.
fn load_plain_image(group: &Group, name: &str) -> Option<GraphicsImage> {
    let data = group.read_file(name).ok()?;
    let format = definition_image_format(Path::new(name))?;
    let image = decode_definition_image(&data, format)?;
    let (width, height) = image.dimensions();
    (width > 0 && height > 0).then(|| GraphicsImage::new(width, height, image.into_raw()))
}

const C4_DEFINITION_GAME_PALETTE: &[u8; 256 * 3] =
    include_bytes!("../../../planet/Graphics.c4g/C4.PAL");

/// `C4GraphicsResource::Init` expands C4.PAL's six-bit channels and installs
/// inverse-alpha overrides for the transparent background and force-field
/// blue (src/C4GraphicsResource.cpp:183-193). Convert that packed palette to
/// conventional RGBA at the same point C4Surface::SetPix resolves an index.
fn definition_game_palette_pixel(index: u8) -> [u8; 4] {
    if index == 0 {
        return [0, 0, 0, 0];
    }
    if index == 191 {
        return [0, 0, 255, 128];
    }
    let offset = usize::from(index) * 3;
    [
        C4_DEFINITION_GAME_PALETTE[offset] << 2,
        C4_DEFINITION_GAME_PALETTE[offset + 1] << 2,
        C4_DEFINITION_GAME_PALETTE[offset + 2] << 2,
        255,
    ]
}

fn definition_image_format(path: &Path) -> Option<image::ImageFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some(image::ImageFormat::Png),
        Some("bmp") => Some(image::ImageFormat::Bmp),
        Some("jpg" | "jpeg") => Some(image::ImageFormat::Jpeg),
        Some("tga") => Some(image::ImageFormat::Tga),
        _ => None,
    }
}

fn definition_image_format_bytes(filename: &[u8]) -> Option<image::ImageFormat> {
    let (_, extension) = split_legacy_extension(filename)?;
    if extension.eq_ignore_ascii_case(b"png") {
        Some(image::ImageFormat::Png)
    } else if extension.eq_ignore_ascii_case(b"bmp") {
        Some(image::ImageFormat::Bmp)
    } else if extension.eq_ignore_ascii_case(b"jpg") || extension.eq_ignore_ascii_case(b"jpeg") {
        Some(image::ImageFormat::Jpeg)
    } else if extension.eq_ignore_ascii_case(b"tga") {
        Some(image::ImageFormat::Tga)
    } else {
        None
    }
}

/// C4Surface::Read(..., fOwnPal=false) ignores an 8-bit BMP's embedded color
/// table and resolves every index through the game palette initialized from
/// C4.PAL. Keep the generic image decoder for C++'s separate 24-bit BMP path
/// and other image formats. A recognized but invalid 8-bit BMP must not fall
/// back because the indexed decoder carries the intentional native-layout
/// hardening.
fn decode_definition_image(data: &[u8], format: image::ImageFormat) -> Option<image::RgbaImage> {
    decode_definition_image_result(data, format).ok()
}

fn decode_definition_image_result(
    data: &[u8],
    format: image::ImageFormat,
) -> Result<image::RgbaImage, String> {
    let bit_count = data
        .get(28..30)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
    if format == image::ImageFormat::Bmp && bit_count == Some(8) {
        let bitmap = IndexedBitmap::decode(data).map_err(|error| error.to_string())?;
        let pixels = bitmap
            .indices
            .iter()
            .flat_map(|index| definition_game_palette_pixel(*index))
            .collect();
        return image::RgbaImage::from_raw(bitmap.width, bitmap.height, pixels)
            .ok_or_else(|| "indexed bitmap dimensions do not match its pixel data".to_string());
    }
    crate::load_image_from_memory_with_format(data, format)
        .map(|image| image.into_rgba8())
        .map_err(|error| error.to_string())
}

/// `C4MaxPhysical` (C4InfoCore.h:31): the 100% value of every physical.
pub const C4_MAX_PHYSICAL: i32 = 100_000;

/// Mirror of `C4PhysicalInfo` (C4InfoCore.h:34-63), parsed from the
/// `[Physical]` section of DefCore.txt with the `C4PhysInfoNameMap` field
/// names (C4InfoCore.cpp:181-205). Defaults are all zero
/// (`C4PhysicalInfo::Default`, C4InfoCore.cpp:239-242).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PhysicalInfo {
    pub energy: i32,
    pub breath: i32,
    pub walk: i32,
    pub jump: i32,
    pub scale: i32,
    pub hangle: i32,
    pub dig: i32,
    pub swim: i32,
    pub throw: i32,
    pub push: i32,
    pub fight: i32,
    pub magic: i32,
    pub float: i32,
    pub can_scale: i32,
    pub can_hangle: i32,
    pub can_dig: i32,
    pub can_construct: i32,
    pub can_chop: i32,
    pub can_fly: i32,
    pub corrosion_resist: i32,
    pub breathe_water: i32,
}

impl PhysicalInfo {
    /// A mutable slot by its `C4PhysInfoNameMap` name (the C++
    /// `GetOffsetByName`, C4InfoCore.cpp:181-205; case-insensitive); None
    /// for unknown names.
    pub fn value_mut_by_name(&mut self, name: &str) -> Option<&mut i32> {
        match name.to_ascii_lowercase().as_str() {
            "energy" => Some(&mut self.energy),
            "breath" => Some(&mut self.breath),
            "walk" => Some(&mut self.walk),
            "jump" => Some(&mut self.jump),
            "scale" => Some(&mut self.scale),
            "hangle" => Some(&mut self.hangle),
            "dig" => Some(&mut self.dig),
            "swim" => Some(&mut self.swim),
            "throw" => Some(&mut self.throw),
            "push" => Some(&mut self.push),
            "fight" => Some(&mut self.fight),
            "magic" => Some(&mut self.magic),
            "float" => Some(&mut self.float),
            "canscale" => Some(&mut self.can_scale),
            "canhangle" => Some(&mut self.can_hangle),
            "candig" => Some(&mut self.can_dig),
            "canconstruct" => Some(&mut self.can_construct),
            "canchop" => Some(&mut self.can_chop),
            "canfly" => Some(&mut self.can_fly),
            "corrosionresist" => Some(&mut self.corrosion_resist),
            "breathewater" => Some(&mut self.breathe_water),
            _ => None,
        }
    }

    /// Read a value by its `C4PhysInfoNameMap` name; None for unknown names.
    pub fn value_by_name(&self, name: &str) -> Option<i32> {
        let mut copy = *self;
        copy.value_mut_by_name(name).map(|slot| *slot)
    }

    /// Assign a value by its `C4PhysInfoNameMap` name; returns false for
    /// unknown names.
    pub fn set_by_name(&mut self, name: &str, value: i32) -> bool {
        self.value_mut_by_name(name)
            .map(|slot| *slot = value)
            .is_some()
    }

    /// `C4PhysicalInfo::TrainValue` (C4InfoCore.cpp:279-285): only nonzero
    /// values train; never above the cap, never decreased.
    pub fn train_value(value: &mut i32, train_by: i32, max_train: i32) {
        if *value != 0 {
            *value = (*value + train_by).min(max_train).max(*value);
        }
    }
}

/// Parsed metadata from `DefCore.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefComponent {
    pub id: String,
    pub count: i32,
}

#[derive(Debug, Clone)]
pub struct DefCore {
    pub id: String,
    /// Five-component `rC4XVer` loaded from DefCore `Version=`
    /// (src/C4Def.cpp:124,254).
    pub version: [i32; 5],
    /// `None` means the key was absent; `Some("")` preserves an explicit
    /// empty `Name=` instead of applying C++'s missing-value default.
    pub name: Option<String>,
    /// DefCore `RequireDef` C4IDList. Unlike Components, this list carries
    /// IDs only (`mkParAdapt(RequireDef, false)`).
    pub require_defs: Vec<String>,
    pub category: i32,
    pub max_user_select: i32,
    /// Raw DefCore `CrewMember` value. C++ stores this as a signed integer:
    /// gameplay treats any nonzero value as enabled, while FnCrewMember
    /// returns the literal value to script.
    pub crew_member: i32,
    /// DefCore `NoStandardCrew` / C4DefCore::NativeCrew.
    pub no_standard_crew: i32,
    pub value: i32,
    /// `Rebuy` (C4Def.cpp:359): sold objects may introduce their ID into
    /// the player's home-base stock when nonzero.
    pub rebuyable: bool,
    /// `BaseAutoSell` (C4Def.cpp:457): bases automatically sell this object
    /// when BASEFUNC_AutoSellContents is active. GOLD defaults to true.
    pub base_auto_sell: bool,
    /// `NoSell` (C4Def.cpp:411): any nonzero value prevents this definition
    /// from being selected by SellFromBase.
    pub no_sell: i32,
    pub mass: i32,
    /// `MoveToRange` (C4Def.cpp:400): positive values override the command's
    /// default five-pixel arrival range for non-crew objects.
    pub move_to_range: i32,
    /// `Pathfinder` (C4Def.cpp:399): nonzero opts non-crew objects into
    /// C4Command::MoveTo path search and supplies its clamped search level.
    pub pathfinder: i32,
    /// `NoTransferZones` (C4Def.cpp:415): disables transfer-zone edges in
    /// C4Command::MoveTo path search.
    pub no_transfer_zones: i32,
    /// `NoPushEnter` (C4Def.cpp:396): any nonzero value prevents this
    /// definition from executing C4Command::Enter.
    pub no_push_enter: i32,
    pub drag_image_picture: i32,
    pub picture: Option<PictureRect>,
    pub color_by_owner: bool,
    pub color_by_material: String,
    /// DefCore `AllowPictureStack` exceptions to
    /// C4Object::CanConcatPictureWith's picture equality checks.
    pub allow_picture_stack: i32,
    /// DefCore graphics `Scale` as a percent (C4Def.cpp:456,725).
    pub graphics_scale: u32,
    /// Definition-default C4Object::BlitMode (DefCore `BlitMode`).
    pub blit_mode: u32,
    pub shape: Option<PictureRect>,
    /// Shape-relative fire emission offset (C4Shape::FireTop).
    pub fire_top: i32,
    /// `LiftTop` (C4Def.cpp:385): target height above the lifter at which
    /// DFA_LIFT calls the lifter's `LiftTop` callback.
    pub lift_top: i32,
    pub solid_mask: Option<TargetRect>,
    /// `TopFace` (C4Def.cpp:306): source facet plus object-relative draw target.
    pub top_face: Option<TargetRect>,
    pub vertices: Vec<DefVertex>,
    /// Complete C4Shape fixed-slot storage. `vertices` is the active
    /// `VtxNum` prefix; these slots also retain dormant array values that a
    /// later AddVertex can expose without rewriting CNAT/friction.
    pub vertex_slots: [DefVertex; C4D_MAX_VERTEX],
    pub contact_density: i32,
    pub contact_function_calls: bool,
    pub collection: Option<PictureRect>,
    /// Raw signed `CollectionLimit`. Zero is unlimited; every other value
    /// participates in the native signed count comparison, so negatives make
    /// even an empty collector full.
    pub collection_limit: i32,
    /// `Fragile` (C4Def.cpp:393): any nonzero value prevents Put from
    /// choosing the outdoor throw-in path.
    pub fragile: bool,
    /// `Projectile` (C4Def.cpp:395): nonzero definitions are selected by
    /// C4Command::Attack from the attacker's contents.
    pub projectile: i32,
    pub explosive: i32,
    /// ContactIncinerate=N: positive N gives a 1-in-N chance of catching fire
    /// on contact with a burning object (CrossCheck pass 1,
    /// C4GameObjects.cpp:121-125). Incendiary material checks any nonzero
    /// value instead (C4Object.cpp:932-938), including negatives.
    pub contact_incinerate: i32,
    /// BlastIncinerate=N: incinerate when accumulated Damage reaches N after
    /// a blast (C4Object::Blast, C4Object.cpp:1421-1423); 0 = off.
    pub blast_incinerate: i32,
    /// ContainBlast=1: this container shields its contents from explosions
    /// (the DoExplosion container walk, C4Effect.cpp:884; C4Def.cpp:380).
    pub contain_blast: i32,
    /// `ClosedContainer` (C4Def.cpp:403): any nonzero value shields
    /// contained objects from the container's cached material. Value 1 also
    /// blocks the contained object's view while value 2 does not, so retain
    /// the signed integer rather than collapsing it to a bool.
    pub closed_container: i32,
    /// HorizontalFix=1 (C4Def::NoHorizontalMove, C4Def.cpp:383): exempt
    /// from shockwave flings (Game::BlastObjects, C4Game.cpp:1272).
    pub no_horizontal_move: i32,
    /// NoBurnDecay=1: burning does not reduce Con (C4Object.cpp:777-778).
    pub no_burn_decay: bool,
    /// `Float` (C4Def.cpp:379, default 0): buoyancy line offset in percent
    /// of Con — IsInLiquidCheck probes GBackLiquid(x, y + Float*Con/FullCon
    /// - 1) (C4Object.cpp:5609-5612).
    pub float_line: i32,
    /// `Line=` (C4D_Line* type tokens, C4Def.cpp:318-332); Line objects
    /// skip UpdateShape and their vertices span the action targets.
    pub line: i32,
    /// `LineIntersect=` (0 wrapping, 1 direct vertex assignment).
    pub line_intersect: i32,
    /// `Grab` (C4Def.cpp): 0 none, 1 grab+push, 2 grab-only.
    pub grab: i32,
    /// `GrabPutGet` bitfield (src/C4Def.cpp:364-373): C4D_GrabPut=1 |
    /// C4D_GrabGet=2 — the grabbed-vehicle put/get commands.
    pub grab_put_get: i32,
    /// `NoGet` (src/C4Def.cpp:412): any nonzero value hides the definition
    /// from manual get/activate menus. Retain the signed compiler value.
    pub no_get: i32,
    /// `VehicleControl` (src/C4Def.cpp:398, default 0):
    /// C4D_VehicleControl_Outside=1 | C4D_VehicleControl_Inside=2 — the
    /// SetCommand ControlCommand overloads (src/C4Object.cpp:3944-3969).
    pub vehicle_control: i32,
    /// `NoBreath` (C4Def.cpp:409): exempt from the ExecLife breathing check.
    pub no_breath: bool,
    pub temporary_crew: i32,
    /// DefCore `SmokeRate` defaults to 100.
    pub smoke_rate: i32,
    /// NoBurnDamage=1: burning deals no damage (C4Object.cpp:780).
    pub no_burn_damage: bool,
    /// `BurnTo=ID` (`C4Def::BurnTurnTo`): definition change on incineration
    /// (C4Effect.cpp:580-585).
    pub burn_turn_to: Option<String>,
    /// `ConstructTo=ID` (`C4Def::BuildTurnTo`): successful Build ticks
    /// change the construction target to this definition after DoCon.
    pub build_turn_to: Option<String>,
    /// IncompleteActivity=1: keeps contents on incineration and allows
    /// collection below FullCon (C4Effect.cpp:588, SetOCF C4Object.cpp:594).
    pub incomplete_activity: bool,
    /// The [Physical] section (C4Def loads it via FollowName("Physical"),
    /// C4Def.cpp:459-460).
    pub physical: PhysicalInfo,
    pub collectible: bool,
    pub constructable: bool,
    pub con_size_off: i32,
    pub stretch_growth: bool,
    /// `Oversize` (C4Def.cpp:392): DoCon has no upper FullCon clamp when
    /// nonzero; construction-scaled mass, components and shape may exceed
    /// 100 percent.
    pub oversize: bool,
    /// `Placement=` (C4Def.cpp:312): 0 surface, 1 liquid, 2 air —
    /// PlaceVegetation/PlaceAnimal dispatch on it (C4Game.cpp:2978,3034).
    pub placement: i32,
    /// `Growth=` (C4Def.cpp:358): growth speed; non-zero admits the
    /// random-growth draw in PlaceVegetation (C4Game.cpp:2974).
    pub growth: i32,
    pub basement: i32,
    pub rotateable: i32,
    pub border_bound: i32,
    /// Raw signed `UprightAttach`; C4Object ORs this into Action.t_attach and
    /// C4Shape later consumes its low byte.
    pub upright_attach: i32,
    /// RotatedSolidmasks (C4Def.cpp:414, default 0): solid masks stay put
    /// while the object is rotated (C4Object.cpp:5655).
    pub rotated_solid_masks: bool,
    /// `AutoContextMenu` (C4Def.cpp:416, default 0): entering this container
    /// may automatically open its context menu (C4Object.cpp:2049-2056).
    pub auto_context_menu: bool,
    pub needed_gfx_mode: i32,
    /// `SilentCommands` (C4Def.cpp:404, default 0): suppresses the common
    /// command-failure message, sound, and ComDir stop tail.
    pub silent_commands: bool,
    /// `NoComponentMass` (C4Def compile): contents mass does not add to
    /// the live Mass (C4Object::UpdateMass, C4Object.cpp:497-501).
    pub no_component_mass: bool,
    /// NoStabilize (C4Def.cpp:402): opts out of the Stabilize upright snap.
    pub no_stabilize: bool,
    pub hide_hud_bars: i32,
    pub hide_hud_elements: i32,
    /// Timer= interval in frames (default 35, C4Def.cpp:298).
    pub timer: i32,
    /// TimerCall= function name (C4Def.cpp:299); None when absent/empty.
    pub timer_call: Option<String>,
    pub components: Vec<DefComponent>,
    pub line_connect: u32,
    /// `Entrance` rect (C4Def.cpp:309): the enter/activate area for
    /// OCF_Entrance (SetOCF, C4Object.cpp:584-587).
    pub entrance: Option<PictureRect>,
    /// `RotatedEntrance` (C4Def.cpp:377): 0 = upright only, 1 = any
    /// rotation, N = up to N degrees (SetOCF, C4Object.cpp:586).
    pub rotated_entrance: i32,
    /// `Exclusive` (C4Def.cpp:313): blocks action/construction behind it
    /// (OCF_Exclusive, SetOCF C4Object.cpp:581-583).
    pub exclusive: bool,
    /// `Prey` (C4Def.cpp:354): OCF_Prey while alive (SetOCF,
    /// C4Object.cpp:615-618).
    pub prey: bool,
    /// `Edible` (C4Def.cpp:355): OCF_Edible (SetOCF, C4Object.cpp:630-632).
    pub edible: bool,
    /// `Chop` -> C4Def::Chopable (C4Def.cpp:378): OCF_Chop candidate
    /// (SetOCF, C4Object.cpp:570-575).
    pub chopable: bool,
    /// `AttractLightning` (C4Def.cpp:391): OCF_AttractLightning at FullCon
    /// (SetOCF, C4Object.cpp:623-626).
    pub attract_lightning: bool,
    /// `NoFight` (C4Def.cpp:413): suppresses OCF_FightReady (SetOCF,
    /// C4Object.cpp:606-610).
    pub no_fight: bool,
    /// DefCore `Base` (`C4Def::CanBeBase`): marks structures usable as the
    /// FirstBase in PlaceReadyBase (C4Player.cpp:596-599).
    pub can_be_base: bool,
    /// Signed compiler values that the gameplay projection intentionally
    /// normalizes (mostly int32 flags represented as bools/options). Runtime
    /// reflection must retain these exact post-parse values.
    #[doc(hidden)]
    pub reflected_ints: HashMap<String, i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefVertex {
    pub x: i32,
    pub y: i32,
    pub cnat: u32,
    pub friction: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

impl DefCore {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        Self::load_with_diagnostics(group, ResourceLoadDiagnostic::emit)
    }

    pub fn load_with_diagnostics(
        group: &Group,
        mut report_diagnostic: impl FnMut(ResourceLoadDiagnostic),
    ) -> Result<Self, DefinitionError> {
        let bytes = group
            .load_entry_string("DefCore.txt")
            .map_err(|err| match err {
                GroupError::EntryNotFound(_) | GroupError::EmptyEntry(_) => {
                    DefinitionError::DefCoreMissing
                }
                GroupError::Io(ref io_error) if io_error.kind() == io::ErrorKind::NotFound => {
                    DefinitionError::DefCoreMissing
                }
                other => DefinitionError::Resources(other),
            })?;
        let mut core = parse_def_core_with_diagnostics(&bytes, &mut report_diagnostic)?;

        // C4DefCore::Load adjusts the compiled Category in this order: a
        // signed nonzero CrewMember adds C4D_CrewMember, then a category with
        // no low-five sort bit receives C4D_StaticBack (C4Def.cpp:206-233).
        if core.crew_member != 0 {
            core.category |= C4D_CREW_MEMBER;
        }
        if core.category & C4D_SORT_LIMIT == 0 {
            core.category = (core.category & !C4D_SORT_LIMIT) | 1;
        }
        // C4DefCore::Load replaces a missing or zero-sized Picture with the
        // top-left shape-sized facet after compiling DefCore.txt. Shape
        // offsets are deliberately ignored (C4Def.cpp:221-223).
        if core
            .picture
            .is_none_or(|picture| picture.width == 0 || picture.height == 0)
        {
            let (width, height) = core
                .shape
                .map_or((0, 0), |shape| (shape.width, shape.height));
            core.picture = Some(PictureRect {
                x: 0,
                y: 0,
                width,
                height,
            });
        }

        Ok(core)
    }

    /// `LooksLikeID(C4ID)` after C4IDAdapt has compiled the four-byte token.
    pub fn has_valid_id(&self) -> bool {
        looks_like_compiled_c4id(&self.id)
    }
}

fn looks_like_compiled_c4id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.len() != 4 || id == "NONE" {
        return false;
    }
    if bytes.iter().all(u8::is_ascii_digit) {
        return bytes != b"0000";
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// Combined script sources originating from a definition group.
#[derive(Debug, Clone)]
pub struct DefinitionScript {
    files: Vec<DefinitionScriptFile>,
    combined: String,
    /// Loaded alongside the other definition text components. Keeping this
    /// private preserves `Definition`'s public resource-parts shape while the
    /// accessor above exposes the C4Def-level description.
    definition_description: Option<String>,
}

impl DefinitionScript {
    pub fn files(&self) -> &[DefinitionScriptFile] {
        &self.files
    }

    pub fn combined(&self) -> &str {
        &self.combined
    }
}

/// Individual script source file.
#[derive(Debug, Clone)]
pub struct DefinitionScriptFile {
    pub path: PathBuf,
    pub contents: String,
}

/// Rectangle metadata parsed from `Picture=` in `DefCore.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `DFA_NONE` (C4Def.h:429): no procedure.
pub const DFA_NONE: i32 = -1;
/// `ActIdle` (C4Def.h:139): no next action.
pub const ACT_IDLE: i32 = -1;
/// `ActHold` (C4Def.h:140): hold the final phase.
pub const ACT_HOLD: i32 = -2;

/// `ProcedureName` table (C4Def.cpp:38-58); index = DFA_* value
/// (C4Def.h:430-447). Matching is case-SENSITIVE (`SEqual`,
/// C4Def.cpp:781).
pub const PROCEDURE_NAMES: [&str; 18] = [
    "WALK", "FLIGHT", "KNEEL", "SCALE", "HANGLE", "DIG", "SWIM", "THROW", "BRIDGE", "BUILD",
    "PUSH", "CHOP", "LIFT", "FLOAT", "ATTACH", "FIGHT", "CONNECT", "PULL",
];

/// Representation of `ActMap.txt`. Actions keep their file order and
/// duplicates, mirroring the C++ `C4ActionDef` array — `NextAction` indices
/// and first-match name lookups (`SetActionByName`) depend on it.
#[derive(Debug, Clone)]
pub struct ActionMap {
    pub default_action: Option<String>,
    pub actions: Vec<(String, ActionDefinition)>,
}

impl ActionMap {
    /// First action with the given name, like C++ `SetActionByName`'s forward
    /// scan (C4Object.cpp).
    pub fn get(&self, name: &str) -> Option<&ActionDefinition> {
        self.actions
            .iter()
            .find(|(action_name, _)| action_name == name)
            .map(|(_, action)| action)
    }
}

/// Action metadata used to construct runtime action specifications.
#[derive(Debug, Clone)]
pub struct ActionDefinition {
    pub procedure: Option<String>,
    /// Numeric DFA_* procedure from `CrossMapActMap` (C4Def.cpp:778-782);
    /// `DFA_NONE` when ProcedureName has no case-sensitive table match.
    pub procedure_index: i32,
    /// Signed `C4ActionDef::Length` (default 1).
    pub length: Option<i32>,
    pub next_action: Option<String>,
    /// Numeric next action from `CrossMapActMap` (C4Def.cpp:783-792):
    /// `ACT_IDLE`, `ACT_HOLD`, or an index into `ActionMap::actions`.
    pub next_action_index: i32,
    /// `InLiquidAction` (C4ActionDef; the ExecAction head switches to it
    /// while InLiquid, C4Object.cpp:4749-4753).
    pub in_liquid_action: Option<String>,
    /// Signed `C4ActionDef::Delay` (default 0). Negative values are odd but
    /// valid compiler input and make the phase-delay comparison succeed.
    pub delay: Option<i32>,
    /// Signed `C4ActionDef::Step` (default 1). Zero freezes the phase and a
    /// negative value runs it backwards.
    pub step: Option<i32>,
    pub phase_call: Option<String>,
    pub start_call: Option<String>,
    pub end_call: Option<String>,
    pub abort_call: Option<String>,
    /// Raw `Sound` identifier reflected by `GetActMapVal`.
    pub sound: Option<String>,
    pub no_other_action: bool,
    /// `ObjectDisabled=` (C4ActionDef::Disabled, C4Def.cpp:106): the
    /// action suspends the object — vetoes OCF_Collection/OCF_FightReady
    /// (SetOCF, C4Object.cpp:597,608).
    pub disabled: bool,
    /// Signed `EnergyUsage=` consumed by ExecAction while
    /// C4RULE_StructuresNeedEnergy is active (C4Def.cpp:108;
    /// C4Object.cpp:4738-4753).
    pub energy_usage: i32,
    pub dig_free: Option<i32>,
    pub attach: u32,
    /// Signed `C4ActionDef::Directions` (default 1).
    pub directions: Option<i32>,
    /// `TurnAction` (C4ActionDef): SetDir fires it on direction change
    /// (C4Object.cpp:4225-4240).
    pub turn_action: Option<String>,
    /// Signed `C4ActionDef::FlipDir` (default 0).
    pub flip_dir: Option<i32>,
    pub facet: Option<ActionFacet>,
    pub reverse: bool,
    pub facet_base: bool,
    pub facet_top_face: bool,
    pub facet_target_stretch: bool,
    /// Exact signed C4ActionDef compiler values used to preserve non-boolean
    /// payloads for fields whose modeled runtime projection is boolean.
    pub reflected_ints: HashMap<String, i32>,
}

impl Default for ActionDefinition {
    fn default() -> Self {
        Self {
            procedure: None,
            // C4ActionDef ctor: Procedure{DFA_NONE} (C4Def.cpp:62),
            // NextAction{ActIdle} (C4Def.h:154).
            procedure_index: DFA_NONE,
            length: None,
            in_liquid_action: None,
            next_action: None,
            next_action_index: ACT_IDLE,
            delay: None,
            step: None,
            phase_call: None,
            start_call: None,
            end_call: None,
            abort_call: None,
            sound: None,
            no_other_action: false,
            disabled: false,
            energy_usage: 0,
            dig_free: None,
            attach: 0,
            turn_action: None,
            directions: None,
            flip_dir: None,
            facet: None,
            reverse: false,
            facet_base: false,
            facet_top_face: false,
            facet_target_stretch: false,
            reflected_ints: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

#[derive(Debug, Error)]
pub enum DefinitionError {
    #[error("definition core `DefCore.txt` missing")]
    DefCoreMissing,
    #[error("definition core is missing required field `{0}`")]
    MissingDefCoreField(&'static str),
    #[error("definition core references unknown category flag `{0}`")]
    UnknownCategoryFlag(String),
    #[error("definition core references unknown line connect flag `{0}`")]
    UnknownLineConnectFlag(String),
    #[error("definition core category value `{0}` is not valid")]
    InvalidCategoryValue(String),
    #[error("definition core could not be parsed: {0}")]
    DefCoreParse(String),
    #[error("definition is missing script sources")]
    ScriptMissing,
    #[error("definition script `{path}` is not valid UTF-8")]
    ScriptEncoding {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("ActMap.txt could not be parsed: {0}")]
    ActMapParse(String),
    #[error("ColorByOwner overlay `{path}` could not be loaded: {reason}")]
    ColorByOwnerOverlay { path: PathBuf, reason: String },
    #[error("definition graphics `{path}` could not be loaded: {reason}")]
    Graphics { path: PathBuf, reason: String },
    #[error(transparent)]
    Resources(#[from] GroupError),
}

/// C4IDAdapt reads a fixed four-byte `RCT_ID` buffer. Identifier input may
/// contain lowercase letters and `-`; `LooksLikeID` validates the packed
/// result separately after this truncating read.
fn parse_c4_id_token(value: &str) -> String {
    clonk_script::c4_string_from_bytes(&read_c4_id_token(value))
}

fn read_c4_id_token(value: &str) -> Vec<u8> {
    clonk_script::c4_string_bytes(value)
        .into_iter()
        .skip_while(|byte| matches!(byte, b' ' | b'\t'))
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .take(4)
        .collect()
}

/// Compile one optional C4ID through the native four-byte `C4IDAdapt`.
///
/// The adapter first reads at most four `RCT_ID` bytes, rejects a shorter
/// token, and only then constructs the raw `C4ID`. Canonicalizing through the
/// raw payload preserves accepted lowercase and `-` bytes without confusing
/// them with ordinary Rust strings, while native zero IDs remain absent.
fn parse_optional_c4_id_adapt(value: &str) -> Option<String> {
    let token = read_c4_id_token(value);
    if token.len() != 4 {
        return None;
    }

    let token = clonk_script::c4_string_from_bytes(&token);
    let raw = clonk_script::c4_id_parse(&token);
    (raw != 0).then(|| clonk_script::c4_id_from_raw(raw))
}

fn is_physical_compiler_key(name: &str) -> bool {
    matches!(
        name,
        "Energy"
            | "Breath"
            | "Walk"
            | "Jump"
            | "Scale"
            | "Hangle"
            | "Dig"
            | "Swim"
            | "Throw"
            | "Push"
            | "Fight"
            | "Magic"
            | "Float"
            | "CanScale"
            | "CanHangle"
            | "CanDig"
            | "CanConstruct"
            | "CanChop"
            | "CanFly"
            | "CorrosionResist"
            | "BreatheWater"
    )
}

fn truncate_c4_string_bytes(value: &str, max_bytes: usize) -> String {
    let bytes = clonk_script::c4_string_bytes(value);
    clonk_script::c4_string_from_bytes(&bytes[..bytes.len().min(max_bytes)])
}

pub(crate) struct IniNameNode<'a> {
    pub(crate) name: &'a str,
    pub(crate) raw_value: &'a str,
    pub(crate) parent: usize,
    indent: usize,
}

pub(crate) fn create_ini_name_tree(text: &str) -> Vec<IniNameNode<'_>> {
    let text = text.split_once('\0').map_or(text, |(head, _)| head);
    let mut nodes = vec![IniNameNode {
        name: "",
        raw_value: "",
        parent: 0,
        indent: 0,
    }];
    let mut current = 0;

    // StdCompilerINIRead::CreateNameTree makes indentation structural before
    // it validates the candidate's closing bracket or equals sign. Preserve
    // that ordering: a malformed named line can still dedent later nodes.
    for raw_line in text.split(['\r', '\n']) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        let bytes = line.as_bytes();
        let section =
            bytes.first() == Some(&b'[') && bytes.get(1).is_some_and(u8::is_ascii_alphabetic);
        if !section && !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        // Values behave as one indentation level deeper so an unindented
        // value remains a child of an unindented section.
        let node_indent = indent.saturating_add(usize::from(!section));

        while current != 0 && nodes[current].indent >= node_indent {
            current = nodes[current].parent;
        }
        let parsed = if section {
            ini_section(line)
        } else {
            ini_value(line)
        };
        let Some((name, raw_value)) = parsed else {
            continue;
        };
        let parent = current;
        nodes.push(IniNameNode {
            name,
            raw_value,
            parent,
            indent: node_indent,
        });
        if section {
            current = nodes.len() - 1;
        }
    }
    nodes
}

fn parse_followed_physical(nodes: &[IniNameNode<'_>]) -> PhysicalInfo {
    let Some(def_core) = nodes
        .iter()
        .position(|node| node.parent == 0 && node.name == "DefCore")
    else {
        return PhysicalInfo::default();
    };
    let Some(next_sibling) = nodes
        .iter()
        .skip(def_core + 1)
        .find(|node| node.parent == 0)
    else {
        return PhysicalInfo::default();
    };
    if next_sibling.name != "Physical" {
        return PhysicalInfo::default();
    }

    // FollowName checks the adjacent node, removes DefCore, and then Name()
    // selects the first matching sibling (which can be an earlier duplicate).
    let physical_node = nodes
        .iter()
        .position(|node| node.parent == 0 && node.name == "Physical")
        .expect("the accepted next sibling is a Physical node");
    let mut physical = PhysicalInfo::default();
    let mut seen_values = HashSet::new();
    for node in nodes.iter().filter(|node| node.parent == physical_node) {
        if !is_physical_compiler_key(node.name) || !seen_values.insert(node.name) {
            continue;
        }
        physical.set_by_name(node.name, parse_i32(node.raw_value.trim()).unwrap_or(0));
    }
    physical
}

/// Parse a `DefCore.txt` body with the ordinary diagnostic sink.
///
/// Public so the resource-text fuzz harness reaches the same entry point
/// production loading uses (clonk-org/clonk-rs#963); `DefCore::load` is the
/// ordinary caller, and `parse_def_core_with_diagnostics` is the form that
/// takes a custom sink.
pub fn parse_def_core(bytes: &[u8]) -> Result<DefCore, DefinitionError> {
    parse_def_core_with_diagnostics(bytes, &mut ResourceLoadDiagnostic::emit)
}

fn parse_def_core_with_diagnostics(
    bytes: &[u8],
    report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic),
) -> Result<DefCore, DefinitionError> {
    // C4DefCore::Compile passes a native C string to StdCompilerINIRead.
    // Preserve every pre-NUL byte through the script string projection.
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    let text = clonk_script::c4_string_from_bytes(bytes);
    let name_tree = create_ini_name_tree(&text);
    let def_core_node = name_tree
        .iter()
        .position(|node| node.parent == 0 && node.name == "DefCore");

    let mut id: Option<String> = None;
    let mut version = [0; 5];
    let mut name: Option<String> = None;
    let mut reflected_ints = HashMap::new();
    let mut require_defs = Vec::new();
    let mut category: i32 = 0;
    let mut category_set = false;
    let mut max_user_select: i32 = 0;
    let mut crew_member: i32 = 0;
    let mut no_standard_crew: i32 = 0;
    let mut can_be_base = false;
    let mut object_value: i32 = 0;
    let mut rebuyable = false;
    let mut base_auto_sell: Option<bool> = None;
    let mut no_sell: i32 = 0;
    let mut object_mass: i32 = 0;
    let mut move_to_range: i32 = 0;
    let mut pathfinder: i32 = 0;
    let mut no_transfer_zones: i32 = 0;
    let mut no_push_enter: i32 = 0;
    let mut drag_image_picture: i32 = 0;
    let mut picture: Option<PictureRect> = None;
    let mut color_by_owner = false;
    let mut color_by_material = String::new();
    let mut allow_picture_stack: i32 = 0;
    let mut graphics_scale: u32 = 100;
    let mut blit_mode: u32 = 0;
    let mut shape_width: Option<i32> = None;
    let mut shape_height: Option<i32> = None;
    let mut shape_offset: Option<(i32, i32)> = None;
    let mut fire_top: i32 = 0;
    let mut lift_top: i32 = 0;
    let mut solid_mask: Option<TargetRect> = None;
    let mut top_face: Option<TargetRect> = None;
    let mut vertex_count: usize = 0;
    let mut vertex_x = [0i32; C4D_MAX_VERTEX];
    let mut vertex_y = [0i32; C4D_MAX_VERTEX];
    let mut vertex_cnat = [0u32; C4D_MAX_VERTEX];
    let mut vertex_friction = [0i32; C4D_MAX_VERTEX];
    let mut contact_density: i32 = C4M_SOLID;
    let mut contact_function_calls = false;
    let mut collection: Option<PictureRect> = None;
    let mut collection_limit: i32 = 0;
    let mut fragile = false;
    let mut projectile: i32 = 0;
    let mut explosive: i32 = 0;
    let mut contact_incinerate: i32 = 0;
    let mut blast_incinerate: i32 = 0;
    let mut contain_blast: i32 = 0;
    let mut closed_container: i32 = 0;
    let mut no_horizontal_move: i32 = 0;
    let mut no_burn_decay = false;
    let mut no_breath = false;
    let mut temporary_crew: i32 = 0;
    let mut smoke_rate: i32 = 100;
    let mut grab = 0;
    let mut float_line = 0;
    let mut line_type: i32 = 0;
    let mut line_intersect: i32 = 0;
    let mut no_burn_damage = false;
    let mut burn_turn_to: Option<String> = None;
    let mut build_turn_to: Option<String> = None;
    let mut incomplete_activity = false;
    let physical = parse_followed_physical(&name_tree);
    let mut collectible = false;
    let mut grab_put_get: i32 = 0;
    let mut no_get: i32 = 0;
    let mut vehicle_control: i32 = 0;
    let mut constructable = false;
    let mut con_size_off: i32 = 0;
    let mut stretch_growth = false;
    let mut oversize = false;
    let mut placement: i32 = 0;
    let mut growth: i32 = 0;
    let mut basement: i32 = 0;
    let mut rotateable: i32 = 0;
    let mut border_bound: i32 = 0;
    let mut upright_attach: i32 = 0;
    // RotatedSolidmasks (C4Def.cpp:414, default 0).
    let mut rotated_solid_masks = false;
    // AutoContextMenu (C4Def.cpp:416, default 0).
    let mut auto_context_menu = false;
    let mut needed_gfx_mode: i32 = 0;
    // SilentCommands (C4Def.cpp:404, default 0).
    let mut silent_commands = false;
    let mut no_component_mass = false;
    // NoStabilize (C4Def.cpp:402, default 0): opts out of C4Object::Stabilize.
    let mut no_stabilize = false;
    let mut hide_hud_bars: i32 = 0;
    let mut hide_hud_elements: i32 = 0;
    // Timer=/TimerCall= (C4Def.cpp:298-299): the per-object Def timer.
    let mut timer: i32 = 35;
    let mut timer_call: Option<String> = None;
    let mut components: Vec<DefComponent> = Vec::new();
    let mut line_connect: u32 = 0;
    let mut entrance: Option<PictureRect> = None;
    let mut rotated_entrance: i32 = 0;
    let mut exclusive = false;
    let mut prey = false;
    let mut edible = false;
    let mut chopable = false;
    let mut attract_lightning = false;
    let mut no_fight = false;

    macro_rules! reflected_int {
        ($entry:literal, $value:expr) => {{
            let value = $value;
            reflected_ints.insert($entry.to_string(), value);
            value
        }};
    }

    // Name("DefCore") selects the first exact root child, regardless of
    // whether that node was spelled as a section or value. Each field then
    // selects its first exact direct child and never falls through to a later
    // duplicate after a malformed value.
    let mut seen_values = HashSet::new();
    for node in name_tree
        .iter()
        .filter(|node| Some(node.parent) == def_core_node)
    {
        let key = node.name;
        if !seen_values.insert(key) {
            continue;
        }
        let raw_value = node.raw_value;
        // StdCompilerINIRead::ReadString skips only leading ASCII space/tab
        // for RCT_All. Keep the fully trimmed view for the typed parsers,
        // but preserve trailing bytes for DefCore's three whole-line strings.
        let rct_all_value = raw_value.trim_start_matches([' ', '\t']);
        let value = raw_value.trim();

        match key {
            "id" => {
                if !value.is_empty() {
                    id = Some(parse_c4_id_token(value));
                }
            }
            "Version" => {
                fill_i32_array(value, &mut version);
            }
            "Name" => {
                name = Some(rct_all_value.to_string());
            }
            "RequireDef" => {
                require_defs = parse_id_list(raw_value);
            }
            "MaxUserSelect" => {
                max_user_select = parse_i32(value).unwrap_or(0);
            }
            "Value" => {
                object_value = reflected_int!("Value", parse_i32(value).unwrap_or(0));
            }
            "Rebuy" => {
                rebuyable = reflected_int!("Rebuy", parse_reflected_int(value)) != 0;
            }
            "BaseAutoSell" => {
                base_auto_sell = parse_bool(raw_value);
            }
            "NoSell" => {
                no_sell = parse_i32(value).unwrap_or(0);
            }
            "Mass" => {
                object_mass = parse_i32(value).unwrap_or(0).max(0);
            }
            "MoveToRange" => {
                move_to_range = parse_i32(value).unwrap_or(0);
            }
            "Pathfinder" => {
                pathfinder = parse_i32(value).unwrap_or(0);
            }
            "NoTransferZones" => {
                no_transfer_zones = parse_i32(value).unwrap_or(0);
            }
            "NoPushEnter" => {
                no_push_enter = parse_i32(value).unwrap_or(0);
            }
            "DragImagePicture" => {
                drag_image_picture = parse_i32(value).unwrap_or(0);
            }
            "Category" => {
                category = parse_category(value, report_diagnostic);
                category_set = true;
            }
            "CrewMember" => {
                crew_member = parse_i32(value).unwrap_or(0);
            }
            "NoStandardCrew" => {
                no_standard_crew = parse_i32(value).unwrap_or(0);
            }
            "Base" => {
                can_be_base = reflected_int!("Base", parse_reflected_int(value)) != 0;
            }
            "Picture" => {
                if let Some(rect) = parse_rect(value) {
                    picture = Some(rect);
                }
            }
            "ColorByOwner" => {
                color_by_owner = reflected_int!("ColorByOwner", parse_reflected_int(value)) != 0;
            }
            "ColorByMaterial" => {
                color_by_material = truncate_c4_string_bytes(rct_all_value, 15);
            }
            "AllowPictureStack" => {
                // StdBitfieldAdapt over the APS_* table
                // (src/C4Def.cpp:419-429); numeric values pass through.
                allow_picture_stack = parse_named_bitfield(
                    value,
                    &[
                        ("APS_Color", APS_COLOR),
                        ("APS_Graphics", APS_GRAPHICS),
                        ("APS_Name", APS_NAME),
                        ("APS_Overlay", APS_OVERLAY),
                    ],
                    report_diagnostic,
                );
            }
            "Scale" => {
                let raw = parse_u32(value).unwrap_or(100);
                reflected_ints.insert("Scale".to_string(), raw as i32);
                graphics_scale = raw;
            }
            "BlitMode" => {
                blit_mode = parse_i32(value).unwrap_or(0) as u32;
            }
            // C4Def::CompileFunc maps Width/Height/Offset straight into
            // Shape.Wdt/Hgt/x/y (C4Def.cpp).
            "Width" => {
                shape_width = parse_i32(value);
            }
            "Height" => {
                shape_height = parse_i32(value);
            }
            "Offset" => {
                let mut parts = parse_int_array(value);
                shape_offset = Some((parts.next().unwrap_or(0), parts.next().unwrap_or(0)));
            }
            "FireTop" => {
                fire_top = parse_i32(value).unwrap_or(0);
            }
            "LiftTop" => {
                lift_top = parse_i32(value).unwrap_or(0);
            }
            "SolidMask" => {
                solid_mask = parse_target_rect(value);
            }
            "TopFace" => {
                top_face = parse_target_rect(value);
            }
            "Vertices" => {
                let raw = reflected_int!("Vertices", parse_i32(value).unwrap_or(0));
                vertex_count = raw.clamp(0, C4D_MAX_VERTEX as i32) as usize;
            }
            "VertexX" => {
                fill_i32_array(value, &mut vertex_x);
            }
            "VertexY" => {
                fill_i32_array(value, &mut vertex_y);
            }
            "VertexCNAT" => {
                fill_u32_array(value, &mut vertex_cnat);
            }
            "VertexFriction" => {
                fill_i32_array(value, &mut vertex_friction);
            }
            "ContactDensity" => {
                contact_density = parse_i32(value).unwrap_or(C4M_SOLID);
            }
            "ContactCalls" => {
                contact_function_calls =
                    reflected_int!("ContactCalls", parse_reflected_int(value)) != 0;
            }
            "Collection" => {
                collection = parse_rect(value);
            }
            "Fragile" => {
                fragile = reflected_int!("Fragile", parse_reflected_int(value)) != 0;
            }
            "Projectile" => {
                projectile = parse_i32(value).unwrap_or(0);
            }
            "Explosive" => {
                explosive = parse_i32(value).unwrap_or(0);
            }
            "ContactIncinerate" => {
                contact_incinerate =
                    reflected_int!("ContactIncinerate", parse_i32(value).unwrap_or(0));
            }
            "BlastIncinerate" => {
                blast_incinerate = parse_i32(value).unwrap_or(0);
            }
            "ContainBlast" => {
                contain_blast = parse_i32(value).unwrap_or(0);
            }
            "ClosedContainer" => {
                closed_container = parse_i32(value).unwrap_or(0);
            }
            "HorizontalFix" => {
                no_horizontal_move = parse_i32(value).unwrap_or(0);
            }
            "NoBurnDecay" => {
                no_burn_decay = reflected_int!("NoBurnDecay", parse_reflected_int(value)) != 0;
            }
            "NoBreath" => {
                no_breath = reflected_int!("NoBreath", parse_reflected_int(value)) != 0;
            }
            "TemporaryCrew" => {
                temporary_crew = parse_i32(value).unwrap_or(0);
            }
            "SmokeRate" => {
                smoke_rate = parse_i32(value).unwrap_or(100);
            }
            "Line" => {
                line_type = parse_line_type(value, report_diagnostic);
            }
            "LineIntersect" => {
                line_intersect = parse_i32(value).unwrap_or(0);
            }
            "Float" => {
                float_line = parse_i32(value).unwrap_or(0);
            }
            "Grab" => {
                grab = reflected_int!("Grab", parse_i32(value).unwrap_or(0));
            }
            "VehicleControl" => {
                // Plain integer compile (src/C4Def.cpp:398).
                vehicle_control = parse_i32(value).unwrap_or(0);
            }
            "GrabPutGet" => {
                // StdBitfieldAdapt over C4D_GrabPut/C4D_GrabGet tokens
                // (src/C4Def.cpp:364-373); numeric values pass through.
                grab_put_get = parse_named_bitfield(
                    value,
                    &[("C4D_GrabGet", 2), ("C4D_GrabPut", 1)],
                    report_diagnostic,
                );
            }
            "NoBurnDamage" => {
                no_burn_damage = reflected_int!("NoBurnDamage", parse_reflected_int(value)) != 0;
            }
            "BurnTo" => {
                burn_turn_to = parse_optional_c4_id_adapt(raw_value);
            }
            "ConstructTo" => {
                build_turn_to = parse_optional_c4_id_adapt(raw_value);
            }
            "IncompleteActivity" => {
                incomplete_activity =
                    reflected_int!("IncompleteActivity", parse_reflected_int(value)) != 0;
            }
            "CollectionLimit" => {
                collection_limit = reflected_int!("CollectionLimit", parse_i32(value).unwrap_or(0));
            }
            "Collectible" => {
                collectible = reflected_int!("Collectible", parse_reflected_int(value)) != 0;
            }
            "NoGet" => {
                no_get = reflected_int!("NoGet", parse_i32(value).unwrap_or(0));
            }
            "Construction" => {
                constructable = reflected_int!("Construction", parse_reflected_int(value)) != 0;
            }
            "ConSizeOff" => {
                con_size_off = reflected_int!("ConSizeOff", parse_i32(value).unwrap_or(0));
            }
            "StretchGrowth" => {
                stretch_growth = reflected_int!("StretchGrowth", parse_reflected_int(value)) != 0;
            }
            "Oversize" => {
                // C4Compiler stores this BOOL through an integer adapter;
                // every nonzero value is true, not just the conventional 1.
                oversize = reflected_int!("Oversize", parse_reflected_int(value)) != 0;
            }
            "Placement" => {
                placement = parse_i32(value).unwrap_or(0);
            }
            "Growth" => {
                growth = parse_i32(value).unwrap_or(0);
            }
            "Basement" => {
                basement = reflected_int!("Basement", parse_i32(value).unwrap_or(0));
            }
            "Rotate" => {
                rotateable = reflected_int!("Rotate", parse_i32(value).unwrap_or(0));
            }
            "BorderBound" => {
                border_bound = reflected_int!("BorderBound", parse_i32(value).unwrap_or(0));
            }
            "UprightAttach" => {
                upright_attach = reflected_int!("UprightAttach", parse_i32(value).unwrap_or(0));
            }
            "RotatedSolidmasks" => {
                rotated_solid_masks =
                    reflected_int!("RotatedSolidmasks", parse_reflected_int(value)) != 0;
            }
            "AutoContextMenu" => {
                auto_context_menu =
                    reflected_int!("AutoContextMenu", parse_reflected_int(value)) != 0;
            }
            "NeededGfxMode" => {
                needed_gfx_mode = parse_i32(value).unwrap_or(0);
            }
            "SilentCommands" => {
                silent_commands = reflected_int!("SilentCommands", parse_reflected_int(value)) != 0;
            }
            "NoComponentMass" => {
                no_component_mass =
                    reflected_int!("NoComponentMass", parse_reflected_int(value)) != 0;
            }
            "NoStabilize" => {
                no_stabilize = reflected_int!("NoStabilize", parse_reflected_int(value)) != 0;
            }
            "HideHUDBars" => {
                hide_hud_bars = parse_named_bitfield(
                    value,
                    &[("Energy", 1), ("MagicEnergy", 2), ("Breath", 4), ("All", 7)],
                    report_diagnostic,
                );
            }
            "HideHUDElements" => {
                hide_hud_elements = parse_named_bitfield(
                    value,
                    &[
                        ("Portrait", 1),
                        ("Captain", 2),
                        ("Name", 4),
                        ("Rank", 8),
                        ("RankImage", 16),
                        ("Inventory", 32),
                        ("All", 63),
                    ],
                    report_diagnostic,
                );
            }
            "Timer" => {
                timer = parse_i32(value).unwrap_or(35);
            }
            "TimerCall" => {
                if !rct_all_value.is_empty() {
                    timer_call = Some(truncate_c4_string_bytes(rct_all_value, 29));
                }
            }
            "Components" => {
                components = parse_components(raw_value);
            }
            "LineConnect" => {
                line_connect = parse_line_connect(value, report_diagnostic);
            }
            // C4Object::SetOCF DefCore inputs (C4Def.cpp:309-413).
            "Entrance" => {
                entrance = parse_rect(value);
            }
            "RotatedEntrance" => {
                rotated_entrance = parse_i32(value).unwrap_or(0);
            }
            "Exclusive" => {
                exclusive = reflected_int!("Exclusive", parse_reflected_int(value)) != 0;
            }
            "Prey" => {
                prey = reflected_int!("Prey", parse_reflected_int(value)) != 0;
            }
            "Edible" => {
                edible = reflected_int!("Edible", parse_reflected_int(value)) != 0;
            }
            "Chop" => {
                chopable = reflected_int!("Chop", parse_reflected_int(value)) != 0;
            }
            "AttractLightning" => {
                attract_lightning =
                    reflected_int!("AttractLightning", parse_reflected_int(value)) != 0;
            }
            "NoFight" => {
                no_fight = reflected_int!("NoFight", parse_reflected_int(value)) != 0;
            }
            _ => {}
        }
    }

    let id = id.ok_or(DefinitionError::MissingDefCoreField("id"))?;
    if !category_set {
        // Preserve compatibility with the C++ engine where unspecified category defaults to 0.
        category = 0;
    }

    let vertex_slots = std::array::from_fn(|idx| DefVertex {
        x: vertex_x[idx],
        y: vertex_y[idx],
        cnat: vertex_cnat[idx],
        friction: vertex_friction[idx],
    });
    let vertices = vertex_slots[..vertex_count].to_vec();

    let base_auto_sell = base_auto_sell.unwrap_or_else(|| id.eq_ignore_ascii_case("GOLD"));

    Ok(DefCore {
        id,
        version,
        name,
        require_defs,
        category,
        max_user_select,
        crew_member,
        no_standard_crew,
        value: object_value,
        rebuyable,
        base_auto_sell,
        no_sell,
        mass: object_mass,
        move_to_range,
        pathfinder,
        no_transfer_zones,
        no_push_enter,
        drag_image_picture,
        picture,
        color_by_owner,
        color_by_material,
        allow_picture_stack,
        graphics_scale,
        blit_mode,
        shape: (shape_width.is_some() || shape_height.is_some() || shape_offset.is_some()).then(
            || {
                let (x, y) = shape_offset.unwrap_or((0, 0));
                PictureRect {
                    x,
                    y,
                    width: shape_width.unwrap_or(0),
                    height: shape_height.unwrap_or(0),
                }
            },
        ),
        fire_top,
        lift_top,
        solid_mask,
        top_face,
        vertices,
        vertex_slots,
        contact_density,
        contact_function_calls,
        collection,
        collection_limit,
        fragile,
        projectile,
        explosive,
        contact_incinerate,
        blast_incinerate,
        contain_blast,
        closed_container,
        no_horizontal_move,
        no_burn_decay,
        no_breath,
        temporary_crew,
        smoke_rate,
        grab,
        float_line,
        line: line_type,
        line_intersect,
        no_burn_damage,
        burn_turn_to,
        build_turn_to,
        incomplete_activity,
        physical,
        collectible,
        grab_put_get,
        no_get,
        vehicle_control,
        constructable,
        con_size_off,
        stretch_growth,
        oversize,
        placement,
        growth,
        basement,
        rotateable,
        border_bound,
        upright_attach,
        rotated_solid_masks,
        auto_context_menu,
        needed_gfx_mode,
        silent_commands,
        no_component_mass,
        no_stabilize,
        hide_hud_bars,
        hide_hud_elements,
        timer,
        timer_call,
        components,
        line_connect,
        can_be_base,
        entrance,
        rotated_entrance,
        exclusive,
        prey,
        edible,
        chopable,
        attract_lightning,
        no_fight,
        reflected_ints,
    })
}

fn parse_components(value: &str) -> Vec<DefComponent> {
    parse_c4id_list(value, true)
}

fn parse_id_list(value: &str) -> Vec<String> {
    parse_c4id_list(value, false)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

/// `C4IDList::CompileFunc`: entries are separated only by `SEP_SEP2` (`;`),
/// `C4IDAdapt` consumes at most four identifier bytes, and an invalid ID
/// throws `NotFound` so the container keeps earlier entries and stops.
fn parse_c4id_list(value: &str, with_values: bool) -> Vec<DefComponent> {
    let bytes = value.as_bytes();
    let mut entries = Vec::new();
    let mut cursor = 0;
    let mut first = true;

    loop {
        if !first {
            skip_c4id_list_whitespace(bytes, &mut cursor);
            if bytes.get(cursor) != Some(&b';') {
                break;
            }
            cursor += 1;
        }
        first = false;

        skip_c4id_list_whitespace(bytes, &mut cursor);
        let id_start = cursor;
        while cursor < bytes.len()
            && cursor - id_start < 4
            && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'_' | b'-'))
        {
            cursor += 1;
        }
        let id = &value[id_start..cursor];
        if !looks_like_compiled_c4id(id) {
            break;
        }

        let mut count = 0;
        if with_values {
            skip_c4id_list_whitespace(bytes, &mut cursor);
            if bytes.get(cursor) == Some(&b'=') {
                cursor += 1;
                if let Some((parsed, consumed)) = parse_action_i32_prefix(&bytes[cursor..]) {
                    count = parsed;
                    cursor += consumed;
                }
            }
        }
        entries.push(DefComponent {
            id: id.to_string(),
            count,
        });
    }

    entries
}

fn skip_c4id_list_whitespace(bytes: &[u8], cursor: &mut usize) {
    while bytes
        .get(*cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        *cursor += 1;
    }
}

fn parse_named_bitfield(
    value: &str,
    names: &[(&str, i32)],
    report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic),
) -> i32 {
    // StdBitfieldAdapt first tries an int32 value, then an RCT_Idtf name.
    // Unknown names only warn and contribute no bits. The outer naming
    // adaptor defaults the whole field to zero if either reader cannot
    // consume a token (StdAdaptors.h:950-986).
    let bytes = clonk_script::c4_string_bytes(value);
    let mut cursor = 0;
    let mut flags = 0;
    loop {
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if let Some((number, consumed)) = parse_action_i32_prefix(&bytes[cursor..]) {
            flags |= number;
            cursor += consumed;
        } else {
            let start = cursor;
            while bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                cursor += 1;
            }
            if cursor == start {
                return 0;
            }
            if let Some(bit) = names
                .iter()
                .find_map(|(name, bit)| (&bytes[start..cursor] == name.as_bytes()).then_some(*bit))
            {
                flags |= bit;
            } else {
                report_diagnostic(ResourceLoadDiagnostic::UnknownDefinitionBitName {
                    bit_name: clonk_script::c4_string_from_bytes(&bytes[start..cursor]),
                });
            }
        }
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'|') {
            break;
        }
        cursor += 1;
    }
    flags
}

pub(crate) fn emit_unknown_definition_bit_name(bit_name: &str) {
    tracing::warn!(%bit_name, "unknown definition bit name");
}

/// `mkBitfieldAdapt(Line, LineTypes)` (C4Def.cpp:319-333): named values
/// separated by `|` are ORed. In particular, legacy DPIP spells the drain
/// value as `C4D_LinePower|C4D_LineSource` (1 | 2 = 3).
fn parse_line_type(value: &str, report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic)) -> i32 {
    parse_named_bitfield(
        value,
        &[
            ("C4D_LinePower", 1),
            ("C4D_LineSource", 2),
            ("C4D_LineDrain", 3),
            ("C4D_LineLightning", 4),
            ("C4D_LineVolcano", 5),
            ("C4D_LineRope", 6),
            ("C4D_LineColored", 7),
            ("C4D_LineVertex", 8),
        ],
        report_diagnostic,
    )
}

fn parse_line_connect(
    value: &str,
    report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic),
) -> u32 {
    parse_named_bitfield(
        value,
        &[
            ("C4D_PowerInput", 1),
            ("C4D_PowerOutput", 1 << 1),
            ("C4D_LiquidInput", 1 << 2),
            ("C4D_LiquidOutput", 1 << 3),
            ("C4D_PowerGenerator", 1 << 4),
            ("C4D_PowerConsumer", 1 << 5),
            ("C4D_LiquidPump", 1 << 6),
            ("C4D_ConnectRope", 1 << 7),
            ("C4D_EnergyHolder", 1 << 8),
        ],
        report_diagnostic,
    ) as u32
}

fn load_scripts<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<DefinitionScript, DefinitionError> {
    // C4Def loads C4CFN_Script through C4ComponentHost::LoadAppend. That
    // template has three top-level segments, and each localized segment
    // independently takes the first candidate that can actually be read in
    // language-sequence order. C4Def ignores a segment's read failures.
    let language_codes = if languages.is_empty() {
        vec![String::new()]
    } else {
        languages
            .iter()
            .map(|language| component_language_code(language.as_ref()))
            .collect()
    };
    let mut candidates = Vec::with_capacity(3);
    if let Ok(data) = group.read_file("Script.c") {
        candidates.push(("Script.c".to_string(), data));
    }
    for stem in ["Script", "C4Script"] {
        for language in &language_codes {
            let candidate = format!("{stem}{language}.c");
            if let Ok(data) = group.read_file(&candidate) {
                candidates.push((candidate, data));
                break;
            }
        }
    }

    let mut files = Vec::with_capacity(candidates.len());
    let mut combined = String::new();
    for (candidate, data) in candidates {
        // LoadAppend copies with SCopy: a NUL ends this component without
        // suppressing later selected components.
        let data = data.split(|byte| *byte == 0).next().unwrap_or_default();
        let contents = clonk_script::c4_string_from_bytes(data);

        // LoadAppend prefixes every selected component, including the first
        // and zero-byte components, with exactly one newline.
        combined.push('\n');
        combined.push_str(&contents);
        files.push(DefinitionScriptFile {
            path: PathBuf::from(candidate),
            contents,
        });
    }

    Ok(DefinitionScript {
        files,
        combined,
        definition_description: None,
    })
}

fn ini_section(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'[') || !bytes.get(1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut cursor = 1;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b']'))
        .then(|| (&line[1..name_end], &line[cursor.saturating_add(1)..]))
}

pub(crate) fn ini_section_name(line: &str) -> Option<&str> {
    ini_section(line).map(|(name, _)| name)
}

pub(crate) fn ini_value(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut cursor = 0;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'=')).then(|| (&line[..name_end], &line[cursor + 1..]))
}

fn parse_act_map(bytes: &[u8]) -> Result<ActionMap, DefinitionError> {
    // StdStrBuf/CreateNameTree consume a native C string. Preserve arbitrary
    // legacy bytes through clonk-script's lossless byte projection and ignore
    // everything after the first NUL like the C++ loader.
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    let action_slots = bytes.iter().filter(|byte| **byte == b'[').count();
    if action_slots == 0 {
        return Err(DefinitionError::ActMapParse(
            "ActMap.txt contains no action slots".to_string(),
        ));
    }
    let text = clonk_script::c4_string_from_bytes(bytes);
    let mut actions: Vec<(String, ActionDefinition)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_definition = ActionDefinition::default();
    let mut compile_current_action = false;
    let mut seen_keys = HashSet::new();
    // Valid INI sections form an indentation tree. Only root [Action]
    // nodes feed mkArrayAdaptS; nested actions and their values are ignored.
    let mut section_stack: Vec<(usize, bool)> = Vec::new();

    // StdCompiler::CreateNameTree terminates a line on either byte, not only
    // LF/CRLF. Old packed groups can therefore contain valid CR-only INI.
    for raw_line in text.split(['\r', '\n']) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        let structural = line.trim_end_matches([' ', '\t', '\r']);
        if structural.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }

        if let Some(section) = ini_section_name(structural) {
            while section_stack
                .last()
                .is_some_and(|(parent_indent, _)| *parent_indent >= indent)
            {
                section_stack.pop();
            }
            let root_section = section_stack.is_empty();
            if root_section {
                if compile_current_action {
                    actions.push((current_name.take().unwrap_or_default(), current_definition));
                }
                current_definition = ActionDefinition::default();
                current_name = None;
                seen_keys.clear();
                compile_current_action = section == "Action";
            }
            section_stack.push((indent, root_section && section == "Action"));
            continue;
        }

        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some((key, raw_value)) = ini_value(line) else {
            continue;
        };
        let value_indent = indent.saturating_add(1);
        while section_stack
            .last()
            .is_some_and(|(parent_indent, _)| *parent_indent >= value_indent)
        {
            section_stack.pop();
        }
        if !compile_current_action || section_stack.len() != 1 || !section_stack[0].1 {
            continue;
        }
        let value = raw_value.trim_start_matches([' ', '\t']);
        if !seen_keys.insert(key.to_string()) {
            continue;
        }

        if key == "Name" {
            if let Some(value) = parse_action_string(value) {
                current_name = Some(value);
            }
            continue;
        }

        match key {
            "Procedure" => {
                current_definition.procedure = parse_action_string(value);
            }
            "Length" => {
                let raw = parse_action_int(value, 1);
                current_definition
                    .reflected_ints
                    .insert("Length".to_string(), raw);
                current_definition.length = Some(raw);
            }
            "NextAction" => {
                current_definition.next_action = parse_action_string(value);
            }
            "InLiquidAction" => {
                current_definition.in_liquid_action = parse_action_string(value);
            }
            "Delay" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("Delay".to_string(), raw);
                current_definition.delay = Some(raw);
            }
            "Step" => {
                let raw = parse_action_int(value, 1);
                current_definition
                    .reflected_ints
                    .insert("Step".to_string(), raw);
                current_definition.step = Some(raw);
            }
            "PhaseCall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.phase_call = parse_action_string(value);
                }
            }
            "StartCall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.start_call = parse_action_string(value);
                }
            }
            "EndCall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.end_call = parse_action_string(value);
                }
            }
            "AbortCall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.abort_call = parse_action_string(value);
                }
            }
            "Sound" => {
                current_definition.sound = parse_action_string(value);
            }
            "NoOtherAction" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("NoOtherAction".to_string(), raw);
                current_definition.no_other_action = raw != 0;
            }
            "ObjectDisabled" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("ObjectDisabled".to_string(), raw);
                current_definition.disabled = raw != 0;
            }
            "EnergyUsage" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("EnergyUsage".to_string(), raw);
                current_definition.energy_usage = raw;
            }
            "DigFree" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("DigFree".to_string(), raw);
                current_definition.dig_free = Some(raw);
            }
            "Attach" => {
                let raw = parse_action_attach(value);
                current_definition
                    .reflected_ints
                    .insert("Attach".to_string(), raw);
                current_definition.attach = raw as u32;
            }
            "TurnAction" => {
                current_definition.turn_action = parse_action_string(value);
            }
            "Directions" => {
                let raw = parse_action_int(value, 1);
                current_definition
                    .reflected_ints
                    .insert("Directions".to_string(), raw);
                current_definition.directions = Some(raw);
            }
            "FlipDir" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("FlipDir".to_string(), raw);
                current_definition.flip_dir = Some(raw);
            }
            "Facet" => {
                current_definition.facet = parse_action_facet(value);
            }
            "Reverse" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("Reverse".to_string(), raw);
                current_definition.reverse = raw != 0;
            }
            "FacetBase" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("FacetBase".to_string(), raw);
                current_definition.facet_base = raw != 0;
            }
            "FacetTopFace" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("FacetTopFace".to_string(), raw);
                current_definition.facet_top_face = raw != 0;
            }
            "FacetTargetStretch" => {
                let raw = parse_action_int(value, 0);
                current_definition
                    .reflected_ints
                    .insert("FacetTargetStretch".to_string(), raw);
                current_definition.facet_target_stretch = raw != 0;
            }
            _ => {}
        }
    }

    if compile_current_action {
        actions.push((current_name.unwrap_or_default(), current_definition));
    }
    // C4Def::LoadActMap allocates SCharCount('[', data) slots, while the INI
    // compiler consumes only real [Action] sections in their own order.
    // Unknown/malformed/comment brackets therefore become default slots at
    // the end rather than being interleaved with compiled actions.
    while actions.len() < action_slots {
        actions.push((String::new(), ActionDefinition::default()));
    }

    cross_map_act_map(&mut actions);

    Ok(ActionMap {
        default_action: None,
        actions,
    })
}

/// `C4Def::CrossMapActMap` (C4Def.cpp:773-799): resolve procedure names to
/// DFA_* indices (case-sensitive against `PROCEDURE_NAMES`) and next-action
/// names to indices ("Hold" case-insensitively to `ACT_HOLD`; the C++
/// overwrite loop makes the last duplicate win). The `*Call="None"` clearing
/// from lines 794-797 already happens during parsing.
fn cross_map_act_map(actions: &mut [(String, ActionDefinition)]) {
    let names: Vec<String> = actions.iter().map(|(name, _)| name.clone()).collect();
    for (_, action) in actions.iter_mut() {
        action.procedure_index = DFA_NONE;
        if let Some(procedure) = &action.procedure {
            for (index, table_name) in PROCEDURE_NAMES.iter().enumerate() {
                if procedure == table_name {
                    action.procedure_index = index as i32;
                }
            }
        }
        action.next_action_index = ACT_IDLE;
        if let Some(next) = &action.next_action {
            if next.eq_ignore_ascii_case("Hold") {
                action.next_action_index = ACT_HOLD;
            } else {
                for (index, name) in names.iter().enumerate() {
                    if next == name {
                        action.next_action_index = index as i32;
                    }
                }
            }
        }
    }
}

/// `C4Rect::Scaled` (src/C4Rect.cpp:37-44) under the definition's
/// `C4Def::Scale` (`C4DefCore::Scale / 100.0f`, src/C4Def.cpp:745). The Picture
/// rect is authored in game units; `C4Def::Picture2Facet` (src/C4Def.cpp:1341)
/// scales it into bitmap space before the facet is set, truncating toward zero
/// exactly as `static_cast<int32_t>(static_cast<float>(val) * scale)` does.
fn scaled_picture_rect(rect: PictureRect, graphics_scale: u32) -> PictureRect {
    let scale = graphics_scale as f32 / 100.0;
    let scaled = |value: i32| (value as f32 * scale) as i32;
    PictureRect {
        x: scaled(rect.x),
        y: scaled(rect.y),
        width: scaled(rect.width),
        height: scaled(rect.height),
    }
}

fn crop_definition_picture(
    core: &DefCore,
    graphics: Option<&GraphicsImage>,
) -> Option<GraphicsImage> {
    let graphics = graphics?;
    let (crop_x, crop_y, crop_w, crop_h) = normalize_crop(
        scaled_picture_rect(core.picture?, core.graphics_scale),
        graphics.width(),
        graphics.height(),
    )?;
    let pixels = extract_rgba_bytes(
        graphics.pixels(),
        graphics.width(),
        crop_x,
        crop_y,
        crop_w,
        crop_h,
    );
    Some(GraphicsImage::new(crop_w, crop_h, pixels))
}

fn crop_definition_picture_mask(
    core: &DefCore,
    picture: Option<&GraphicsImage>,
    mask: Option<&ColorByOwnerMask>,
) -> Option<ColorByOwnerMask> {
    let picture = picture?;
    let mask = mask?;
    let (crop_x, crop_y, crop_w, crop_h) = match core.picture {
        // Overlay.png shares Graphics.png's dimensions (C4Surface.cpp:329), so
        // the owner-color mask takes the same scaled Picture rect as the base.
        Some(rect) => normalize_crop(
            scaled_picture_rect(rect, core.graphics_scale),
            mask.width,
            mask.height,
        )
        .unwrap_or((0, 0, mask.width, mask.height)),
        None => (0, 0, mask.width, mask.height),
    };
    if (crop_w, crop_h) != (picture.width(), picture.height()) {
        return None;
    }

    let pixel_count = usize::try_from(u64::from(mask.width) * u64::from(mask.height)).ok()?;
    let channels = if mask.pixels.len() == pixel_count.checked_mul(4)? {
        4
    } else {
        1
    };
    let mut pixels = Vec::with_capacity((crop_w * crop_h) as usize * channels);
    for row in crop_y..crop_y + crop_h {
        let start = (row * mask.width + crop_x) as usize * channels;
        let end = start + crop_w as usize * channels;
        pixels.extend_from_slice(&mask.pixels[start..end]);
    }
    pixels
        .iter()
        .any(|value| *value != 0)
        .then_some(ColorByOwnerMask {
            width: crop_w,
            height: crop_h,
            pixels,
        })
}

const BASE_GRAPHICS_FILES: [&[u8]; 2] = [b"graphics.png", b"graphics.bmp"];
const C4_MAX_NAME_BYTES: usize = 30;

type LoadedDefinitionGraphics = (
    Option<GraphicsImage>,
    Option<ColorByOwnerMask>,
    HashMap<String, DefinitionGraphicsVariant>,
);

fn load_definition_graphics(
    group: &Group,
    color_by_owner: bool,
) -> Result<LoadedDefinitionGraphics, DefinitionError> {
    let entries = group.entries()?;
    let (png_candidates, bmp_candidates) = collect_graphics_entries(&entries);
    if png_candidates.is_empty() && bmp_candidates.is_empty() {
        return Ok((None, None, HashMap::new()));
    }

    let base_entry = select_base_graphics(&entries);
    let mut base_image = None;
    let mut base_mask = None;
    let mut additional = HashMap::new();

    if let Some(base_entry) = base_entry {
        let (image, mask) = load_graphics_group_entry(group, &entries, base_entry, color_by_owner)?;
        base_image = Some(image);
        base_mask = mask;
    }

    // C4DefGraphics appends every PNG in native group order. A later PNG with
    // the same clipped name is still decoded (and may reject the definition),
    // while first-match lookup continues to expose the earlier node.
    for entry in png_candidates {
        if is_base_graphics_entry(entry) {
            continue;
        }
        let Some(name) = derive_variant_name(&entry.name_bytes) else {
            continue;
        };
        let key = normalize_variant_key(&name);
        let (image, mask) = load_graphics_group_entry(group, &entries, entry, color_by_owner)?;
        additional.entry(key).or_insert(DefinitionGraphicsVariant {
            name,
            image,
            color_by_owner_mask: mask,
        });
    }

    // The BMP pass asks Get(clipped_name) before loading. Therefore any PNG
    // or earlier BMP with the same C4 name suppresses this entry completely.
    for entry in bmp_candidates {
        if is_base_graphics_entry(entry) {
            continue;
        }
        let Some(name) = derive_variant_name(&entry.name_bytes) else {
            continue;
        };
        let key = normalize_variant_key(&name);
        if additional.contains_key(&key) {
            continue;
        }
        let (image, mask) = load_graphics_group_entry(group, &entries, entry, color_by_owner)?;
        additional.insert(
            key,
            DefinitionGraphicsVariant {
                name,
                image,
                color_by_owner_mask: mask,
            },
        );
    }

    Ok((base_image, base_mask, additional))
}

fn load_portrait_graphics(
    group: &Group,
    color_by_owner: bool,
) -> Result<Vec<DefinitionGraphicsVariant>, DefinitionError> {
    let mut portraits = Vec::new();
    let entries = group.entries()?;
    for entry in &entries {
        let Some((stem, extension)) = split_legacy_extension(&entry.name_bytes) else {
            continue;
        };
        let Some(suffix) = strip_ascii_case_prefix_bytes(stem, b"Portrait") else {
            continue;
        };
        if !extension.eq_ignore_ascii_case(b"png") && !extension.eq_ignore_ascii_case(b"bmp") {
            continue;
        }
        let name = clipped_legacy_graphics_name(suffix);
        let (image, mask) = load_graphics_group_entry(group, &entries, entry, color_by_owner)?;
        portraits.push(DefinitionGraphicsVariant {
            name,
            image,
            color_by_owner_mask: mask,
        });
    }
    Ok(portraits)
}

fn collect_graphics_entries(entries: &[GroupEntry]) -> (Vec<&GroupEntry>, Vec<&GroupEntry>) {
    let mut png_entries = Vec::new();
    let mut bmp_entries = Vec::new();
    for entry in entries {
        let Some((stem, extension)) = split_legacy_extension(&entry.name_bytes) else {
            continue;
        };
        if strip_ascii_case_prefix_bytes(stem, b"Graphics").is_none() {
            continue;
        }
        if extension.eq_ignore_ascii_case(b"png") {
            png_entries.push(entry);
        } else if extension.eq_ignore_ascii_case(b"bmp") {
            bmp_entries.push(entry);
        }
    }
    (png_entries, bmp_entries)
}

fn select_base_graphics(entries: &[GroupEntry]) -> Option<&GroupEntry> {
    for name in BASE_GRAPHICS_FILES {
        if let Some(entry) = find_group_entry_by_name(entries, name) {
            return Some(entry);
        }
    }

    None
}

fn is_base_graphics_entry(entry: &GroupEntry) -> bool {
    BASE_GRAPHICS_FILES
        .iter()
        .any(|name| entry.name_bytes.eq_ignore_ascii_case(name))
}

fn load_graphics_entry(
    group: &Group,
    path: &Path,
    color_by_owner: bool,
) -> Result<Option<(GraphicsImage, Option<ColorByOwnerMask>)>, DefinitionError> {
    let entries = group.entries()?;
    let name = path.as_os_str().as_encoded_bytes();
    let Some(entry) = find_group_entry_by_name(&entries, name) else {
        return Ok(None);
    };
    load_graphics_group_entry(group, &entries, entry, color_by_owner).map(Some)
}

fn load_graphics_group_entry(
    group: &Group,
    entries: &[GroupEntry],
    entry: &GroupEntry,
    color_by_owner: bool,
) -> Result<(GraphicsImage, Option<ColorByOwnerMask>), DefinitionError> {
    let path = entry.relative_path.clone();
    let data = group
        .read_entry_bytes_exact(entry)
        .map_err(|error| DefinitionError::Graphics {
            path: path.clone(),
            reason: error.to_string(),
        })?;
    let format = definition_image_format_bytes(&entry.name_bytes).ok_or_else(|| {
        DefinitionError::Graphics {
            path: path.clone(),
            reason: "unsupported image format".to_string(),
        }
    })?;
    let mut image = decode_definition_image_result(&data, format).map_err(|reason| {
        DefinitionError::Graphics {
            path: path.clone(),
            reason,
        }
    })?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(DefinitionError::Graphics {
            path: path.clone(),
            reason: format!("invalid image dimensions {width}x{height}"),
        });
    }
    // C4Surface::ReadPNG/SetPixDw canonicalizes the decoded surface before
    // C4DefGraphics derives a ColorByOwner surface from its blue shades.
    blacken_fully_transparent_rgba(image.as_mut());

    let mask = if color_by_owner {
        load_or_generate_color_by_owner_mask(group, entries, entry, &mut image)?
    } else {
        None
    };

    Ok((GraphicsImage::new(width, height, image.into_raw()), mask))
}

fn split_legacy_extension(name: &[u8]) -> Option<(&[u8], &[u8])> {
    let dot = name.iter().rposition(|byte| *byte == b'.')?;
    Some((&name[..dot], &name[dot + 1..]))
}

fn strip_ascii_case_prefix_bytes<'a>(value: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

fn clipped_legacy_graphics_name(suffix: &[u8]) -> String {
    clonk_script::c4_string_from_bytes(&suffix[..suffix.len().min(C4_MAX_NAME_BYTES)])
}

fn derive_variant_name(filename: &[u8]) -> Option<String> {
    let (stem, _) = split_legacy_extension(filename)?;
    let suffix = strip_ascii_case_prefix_bytes(stem, b"Graphics")?;
    (!suffix.is_empty()).then(|| clipped_legacy_graphics_name(suffix))
}

fn normalize_variant_key(name: &str) -> String {
    crate::material::c4_name_key(name)
}

fn find_group_entry_by_name<'a>(entries: &'a [GroupEntry], name: &[u8]) -> Option<&'a GroupEntry> {
    entries
        .iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(name))
}

fn load_or_generate_color_by_owner_mask(
    group: &Group,
    entries: &[GroupEntry],
    graphics_entry: &GroupEntry,
    image: &mut image::RgbaImage,
) -> Result<Option<ColorByOwnerMask>, DefinitionError> {
    if let Some((path, overlay)) = load_color_by_owner_overlay(group, entries, graphics_entry)? {
        if overlay.dimensions() != image.dimensions() {
            let (image_width, image_height) = image.dimensions();
            let (overlay_width, overlay_height) = overlay.dimensions();
            return Err(DefinitionError::ColorByOwnerOverlay {
                path,
                reason: format!(
                    "size {overlay_width}x{overlay_height} does not match graphics {image_width}x{image_height}"
                ),
            });
        }
        return Ok(extract_mask_from_overlay(&overlay, image));
    }
    Ok(generate_color_by_owner_mask(image))
}

fn load_color_by_owner_overlay(
    group: &Group,
    entries: &[GroupEntry],
    graphics_entry: &GroupEntry,
) -> Result<Option<(PathBuf, image::RgbaImage)>, DefinitionError> {
    let Some(candidate_name) = color_by_owner_overlay_name(&graphics_entry.name_bytes) else {
        return Ok(None);
    };
    let Some(candidate) = find_group_entry_by_name(entries, &candidate_name) else {
        return Ok(None);
    };
    let path = candidate.relative_path.clone();
    let data = match group.read_entry_bytes_exact(candidate) {
        Ok(data) => data,
        Err(GroupError::EntryNotFound(_)) => return Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(DefinitionError::ColorByOwnerOverlay {
                path,
                reason: error.to_string(),
            });
        }
    };
    let overlay =
        image::load_from_memory_with_format(&data, image::ImageFormat::Png).map_err(|error| {
            DefinitionError::ColorByOwnerOverlay {
                path: path.clone(),
                reason: error.to_string(),
            }
        })?;
    let mut overlay = overlay.into_rgba8();
    blacken_fully_transparent_rgba(overlay.as_mut());
    Ok(Some((path, overlay)))
}

/// Returns the exact legacy-byte overlay name passed to C++ `LoadGraphics`.
/// The overlay suffix comes from the complete source filename; only the name
/// stored in `C4AdditionalDefGraphics::Name` is clipped to `C4MaxName`.
fn color_by_owner_overlay_name(graphics_name: &[u8]) -> Option<Vec<u8>> {
    let (stem, extension) = split_legacy_extension(graphics_name)?;
    let suffix = if extension.eq_ignore_ascii_case(b"png") {
        strip_ascii_case_prefix_bytes(stem, b"Graphics")
            .or_else(|| strip_ascii_case_prefix_bytes(stem, b"Portrait"))?
    } else if extension.eq_ignore_ascii_case(b"bmp") && stem.eq_ignore_ascii_case(b"Graphics") {
        &[]
    } else {
        return None;
    };

    let mut overlay = Vec::with_capacity(b"Overlay".len() + suffix.len() + b".png".len());
    overlay.extend_from_slice(b"Overlay");
    overlay.extend_from_slice(suffix);
    overlay.extend_from_slice(b".png");
    Some(overlay)
}

fn extract_mask_from_overlay(
    overlay: &image::RgbaImage,
    base: &image::RgbaImage,
) -> Option<ColorByOwnerMask> {
    let (width, height) = base.dimensions();
    if overlay.dimensions() != (width, height) {
        return None;
    }

    // Overlay.png IS the ClrByOwner surface (C4DefGraphics.cpp:74-94 +
    // C4Surface::SetAsClrByOwnerOf, C4Surface.cpp:320-331). C++ keeps its
    // complete texture and draws it owner-modulated over the unchanged base.
    // Retain all four channels: red is not an ownership key, and alpha is the
    // coverage for the second blit pass.
    if overlay.pixels().any(|pixel| pixel[3] != 0) {
        Some(ColorByOwnerMask {
            width,
            height,
            pixels: overlay.as_raw().clone(),
        })
    } else {
        None
    }
}

fn generate_color_by_owner_mask(image: &mut image::RgbaImage) -> Option<ColorByOwnerMask> {
    let (width, height) = image.dimensions();
    let mut pixels = vec![0u8; (width * height) as usize];
    let mut has_mask = false;

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let pixel = image.get_pixel_mut(x, y);
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            let a = pixel[3];
            let dw = u32::from(a) << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
            if let Some(mask_value) = detect_color_by_owner(dw) {
                pixels[idx] = mask_value;
                pixel[0] = 255;
                pixel[1] = 255;
                pixel[2] = 255;
                pixel[3] = a;
                has_mask = true;
            }
        }
    }

    if has_mask {
        Some(ColorByOwnerMask {
            width,
            height,
            pixels,
        })
    } else {
        None
    }
}

fn detect_color_by_owner(dw_clr: u32) -> Option<u8> {
    const RANGE: i32 = 255;
    const HLSMAX: i32 = RANGE;
    const RGBMAX: i32 = 255;

    let r = ((dw_clr >> 16) & 0xff) as i32;
    let g = ((dw_clr >> 8) & 0xff) as i32;
    let b = (dw_clr & 0xff) as i32;
    let c_max = r.max(g).max(b);
    let c_min = r.min(g).min(b);

    let l = ((c_max + c_min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    let mut h;
    let s;
    if c_max == c_min {
        s = 0;
        h = (HLSMAX * 2) / 3;
    } else {
        if l <= (HLSMAX / 2) {
            s = ((c_max - c_min) * HLSMAX + ((c_max + c_min) / 2)) / (c_max + c_min);
        } else {
            s = ((c_max - c_min) * HLSMAX + ((2 * RGBMAX - c_max - c_min) / 2))
                / (2 * RGBMAX - c_max - c_min);
        }

        let rdelta = ((c_max - r) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);
        let gdelta = ((c_max - g) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);
        let bdelta = ((c_max - b) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);

        if r == c_max {
            h = bdelta - gdelta;
        } else if g == c_max {
            h = (HLSMAX / 3) + rdelta - bdelta;
        } else {
            h = (2 * HLSMAX) / 3 + gdelta - rdelta;
        }
        if h < 0 {
            h += HLSMAX;
        }
        if h > HLSMAX {
            h -= HLSMAX;
        }
    }

    if !(145..=175).contains(&h) || s <= 100 {
        return None;
    }

    Some((dw_clr & 0xff) as u8)
}

fn normalize_crop(
    rect: PictureRect,
    image_width: u32,
    image_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let width = rect.width.max(0) as u32;
    let height = rect.height.max(0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut x = rect.x.max(0) as u32;
    let mut y = rect.y.max(0) as u32;
    if x >= image_width || y >= image_height {
        return None;
    }
    if x + width > image_width {
        x = x.min(image_width.saturating_sub(1));
    }
    if y + height > image_height {
        y = y.min(image_height.saturating_sub(1));
    }
    let crop_width = width.min(image_width - x);
    let crop_height = height.min(image_height - y);
    Some((x, y, crop_width.max(1), crop_height.max(1)))
}

fn extract_rgba_bytes(
    pixels: &[u8],
    stride: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut output = Vec::with_capacity((width * height * 4) as usize);
    for row in y..(y + height) {
        let row_start = ((row * stride) + x) as usize * 4;
        let row_end = row_start + (width as usize * 4);
        output.extend_from_slice(&pixels[row_start..row_end]);
    }
    output
}

fn parse_category(value: &str, report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic)) -> i32 {
    parse_named_bitfield(value, CATEGORY_FLAGS, report_diagnostic)
}

pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'1') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        return Some(true);
    }
    if bytes.first() == Some(&b'0') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        return Some(false);
    }
    if bytes.starts_with(b"true") {
        return Some(true);
    }
    if bytes.starts_with(b"false") {
        return Some(false);
    }
    None
}

fn parse_reflected_int(value: &str) -> i32 {
    parse_i32(value).unwrap_or(0)
}

fn parse_action_string(value: &str) -> Option<String> {
    let value = value.trim_start_matches([' ', '\t']);
    let bytes = clonk_script::c4_string_bytes(value);
    let bytes = &bytes[..bytes.len().min(30)];
    (!bytes.is_empty()).then(|| clonk_script::c4_string_from_bytes(bytes))
}

fn parse_action_int(value: &str, default: i32) -> i32 {
    parse_action_i32(value).unwrap_or(default)
}

fn parse_action_i32(value: &str) -> Option<i32> {
    parse_action_i32_prefix(&clonk_script::c4_string_bytes(value)).map(|(value, _)| value)
}

fn parse_action_attach(value: &str) -> i32 {
    let bytes = clonk_script::c4_string_bytes(value);
    let mut cursor = 0;
    let mut flags = 0;
    loop {
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if let Some((value, consumed)) = parse_action_i32_prefix(&bytes[cursor..]) {
            flags |= value;
            cursor += consumed;
        } else {
            let start = cursor;
            while bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                cursor += 1;
            }
            if cursor == start {
                return 0;
            }
            flags |= match &bytes[start..cursor] {
                b"CNAT_None" => 0,
                b"CNAT_Left" => 1,
                b"CNAT_Right" => 2,
                b"CNAT_Top" => 4,
                b"CNAT_Bottom" => 8,
                b"CNAT_Center" => 16,
                b"CNAT_MultiAttach" => 32,
                b"CNAT_NoCollision" => 64,
                _ => 0,
            };
        }
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'|') {
            break;
        }
        cursor += 1;
    }
    flags
}

pub(crate) fn parse_action_i32_prefix(value: &[u8]) -> Option<(i32, usize)> {
    parse_action_i64_prefix(value).map(|(value, consumed)| (value as i32, consumed))
}

fn parse_action_integer_prefix(value: &[u8]) -> Option<(u128, bool, usize)> {
    let mut cursor = 0;
    while value
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    let number_start = cursor;
    // StdCompilerINIRead chooses base 16 only when the untrimmed number
    // itself starts with 0x. A leading sign therefore keeps base 10, exactly
    // like its strtol call (`-0x10` reads decimal -0 and stops at `x`).
    let radix = if value.get(cursor) == Some(&b'0')
        && value
            .get(cursor + 1)
            .is_some_and(|byte| matches!(byte, b'x' | b'X'))
    {
        cursor += 2;
        16u32
    } else {
        // ReadNum chooses the radix after its horizontal SkipWhitespace, then
        // delegates to strtol/strtoul. Those C readers additionally consume
        // the remaining ASCII whitespace (notably form-feed and vertical-tab).
        while value
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'))
        {
            cursor += 1;
        }
        10u32
    };
    let negative = if radix == 10 {
        match value.get(cursor) {
            Some(b'-') => {
                cursor += 1;
                true
            }
            Some(b'+') => {
                cursor += 1;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    let digits_start = cursor;
    let mut magnitude = 0u128;
    while let Some(digit) = value.get(cursor).and_then(|byte| match byte {
        b'0'..=b'9' => Some(u32::from(*byte - b'0')),
        b'a'..=b'f' if radix == 16 => Some(u32::from(*byte - b'a') + 10),
        b'A'..=b'F' if radix == 16 => Some(u32::from(*byte - b'A') + 10),
        _ => None,
    }) {
        if digit >= radix {
            break;
        }
        magnitude = magnitude
            .saturating_mul(u128::from(radix))
            .saturating_add(u128::from(digit));
        cursor += 1;
    }
    if cursor == digits_start {
        // strtol/strtoul("0x", ..., 16) still consume the leading zero.
        if radix == 16 {
            return Some((0, false, number_start + 1));
        }
        return None;
    }
    Some((magnitude, negative, cursor))
}

fn parse_action_i64_prefix(value: &[u8]) -> Option<(i64, usize)> {
    let (magnitude, negative, consumed) = parse_action_integer_prefix(value)?;
    // strtol saturates to native C `long` and the result is then assigned to
    // int32_t. LP64 uses 64-bit long; Windows LLP64 and 32-bit targets use
    // 32-bit long. The final Rust cast supplies the same modulo narrowing.
    let long_bits = std::mem::size_of::<std::os::raw::c_long>() * 8;
    let long_max = (1u128 << (long_bits - 1)) - 1;
    let long_min_magnitude = 1u128 << (long_bits - 1);
    let signed = if negative {
        if magnitude >= long_min_magnitude {
            -(long_min_magnitude as i128)
        } else {
            -(magnitude as i128)
        }
    } else {
        magnitude.min(long_max) as i128
    };
    Some((signed as i64, consumed))
}

pub(crate) fn parse_action_u64_prefix(value: &[u8]) -> Option<(u64, usize)> {
    let (magnitude, negative, consumed) = parse_action_integer_prefix(value)?;
    let long_bits = std::mem::size_of::<std::os::raw::c_ulong>() * 8;
    let long_max = (1u128 << long_bits) - 1;
    let unsigned = if magnitude > long_max {
        long_max
    } else if negative {
        0u128.wrapping_sub(magnitude) & long_max
    } else {
        magnitude
    };
    Some((unsigned as u64, consumed))
}

fn parse_u32(value: &str) -> Option<u32> {
    parse_action_u64_prefix(&clonk_script::c4_string_bytes(value)).map(|(value, _)| value as u32)
}

fn fill_i32_array(value: &str, target: &mut [i32]) {
    for slot in target.iter_mut() {
        *slot = 0;
    }
    for (slot, parsed) in target.iter_mut().zip(parse_int_array(value)) {
        *slot = parsed;
    }
}

fn fill_u32_array(value: &str, target: &mut [u32]) {
    for slot in target.iter_mut() {
        *slot = 0;
    }
    for (slot, parsed) in target.iter_mut().zip(parse_int_array(value)) {
        *slot = parsed as u32;
    }
}

pub(crate) fn parse_int_array(value: &str) -> impl Iterator<Item = i32> {
    parse_int_array_with_default(value, 0)
}

pub(crate) fn parse_int_array_with_default(value: &str, default: i32) -> impl Iterator<Item = i32> {
    let bytes = clonk_script::c4_string_bytes(value);
    let mut values = Vec::new();
    let mut cursor = 0;
    loop {
        if !values.is_empty() {
            while bytes
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b',') {
                break;
            }
            cursor += 1;
        }
        if let Some((parsed, consumed)) = parse_action_i32_prefix(&bytes[cursor..]) {
            values.push(parsed);
            cursor += consumed;
        } else {
            // StdArrayDefaultAdapt installs this slot's default without
            // advancing the compiler cursor. The next comma check therefore
            // stops the remaining slots when garbage caused the failure.
            values.push(default);
        }
    }
    values.into_iter()
}

fn parse_rect(value: &str) -> Option<PictureRect> {
    let mut parts = parse_int_array(value);
    let x = parts.next().unwrap_or(0);
    let y = parts.next().unwrap_or(0);
    let width = parts.next().unwrap_or(0);
    let height = parts.next().unwrap_or(0);
    Some(PictureRect {
        x,
        y,
        width,
        height,
    })
}

fn parse_target_rect(value: &str) -> Option<TargetRect> {
    let mut parts = parse_int_array(value);
    let x = parts.next().unwrap_or(0);
    let y = parts.next().unwrap_or(0);
    let width = parts.next().unwrap_or(0);
    let height = parts.next().unwrap_or(0);
    let target_x = parts.next().unwrap_or(0);
    let target_y = parts.next().unwrap_or(0);
    Some(TargetRect {
        x,
        y,
        width,
        height,
        target_x,
        target_y,
    })
}

pub(crate) fn parse_i32(value: &str) -> Option<i32> {
    parse_action_i32(value)
}

fn parse_action_facet(value: &str) -> Option<ActionFacet> {
    // Every C4TargetRect component has its own zero default. Empty slots and
    // omitted trailing components therefore keep their position instead of
    // shifting later values left. Keep the shared StdCompiler cursor too:
    // once the comma after a value is missing, Separator() makes all later
    // reads fail even if another comma occurs farther along the string.
    let mut numbers = [0; 6];
    let bytes = clonk_script::c4_string_bytes(value);
    let mut cursor = 0;
    let mut readable = true;
    let component_count = numbers.len();
    for (index, slot) in numbers.iter_mut().enumerate() {
        if !readable {
            continue;
        }
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if let Some((value, consumed)) = parse_action_i32_prefix(&bytes[cursor..]) {
            *slot = value;
            cursor += consumed;
        }
        if index < component_count - 1 {
            while bytes
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b',') {
                cursor += 1;
            } else {
                readable = false;
            }
        }
    }
    let x = numbers[0];
    let y = numbers[1];
    let width = numbers[2];
    let height = numbers[3];
    let target_x = numbers[4];
    let target_y = numbers[5];
    Some(ActionFacet {
        x,
        y,
        width,
        height,
        target_x,
        target_y,
    })
}

const CATEGORY_FLAGS: &[(&str, i32)] = &[
    ("C4D_StaticBack", 1 << 0),
    ("C4D_Structure", 1 << 1),
    ("C4D_Vehicle", 1 << 2),
    ("C4D_Living", 1 << 3),
    ("C4D_Object", 1 << 4),
    ("C4D_Goal", 1 << 5),
    ("C4D_Environment", 1 << 6),
    ("C4D_SelectBuilding", 1 << 7),
    ("C4D_SelectVehicle", 1 << 8),
    ("C4D_SelectMaterial", 1 << 9),
    ("C4D_SelectKnowledge", 1 << 10),
    ("C4D_SelectHomebase", 1 << 11),
    ("C4D_SelectAnimal", 1 << 12),
    ("C4D_SelectNest", 1 << 13),
    ("C4D_SelectInEarth", 1 << 14),
    ("C4D_SelectVegetation", 1 << 15),
    ("C4D_TradeLiving", 1 << 16),
    ("C4D_Magic", 1 << 17),
    ("C4D_CrewMember", 1 << 18),
    ("C4D_Rule", 1 << 19),
    ("C4D_Background", 1 << 20),
    ("C4D_Parallax", 1 << 21),
    ("C4D_MouseSelect", 1 << 22),
    ("C4D_Foreground", 1 << 23),
    ("C4D_MouseIgnore", 1 << 24),
    ("C4D_IgnoreFoW", 1 << 25),
];

#[cfg(test)]
mod tests {
    macro_rules! check_eq {
        ($left:expr => $right:expr) => {
            assert_eq!($left, $right);
        };
        ($left:expr => $right:expr, $($message:tt)+) => {
            assert_eq!($left, $right, $($message)+);
        };
    }

    macro_rules! check {
        ($condition:expr) => {
            assert!($condition);
        };
        ($condition:expr, $($message:tt)+) => {
            assert!($condition, $($message)+);
        };
    }

    macro_rules! check_ne {
        ($left:expr => $right:expr) => {
            assert_ne!($left, $right);
        };
        ($left:expr => $right:expr, $($message:tt)+) => {
            assert_ne!($left, $right, $($message)+);
        };
    }

    macro_rules! write_fixture {
        ($path:expr => $contents:expr, $message:expr) => {
            fs::write($path, $contents).expect($message)
        };
        ($path:expr => $contents:expr) => {
            fs::write($path, $contents).unwrap()
        };
    }

    macro_rules! definition_fixture_dir {
        ($temp:ident, $directory:ident => $name:expr) => {
            let $temp = tempdir().expect("tempdir");
            let $directory = $temp.path().join($name);
            fs::create_dir(&$directory).expect("definition directory");
        };
    }

    #[test]
    fn parse_def_core_complete_reflection_only_entries_and_cpp_defaults() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=TST1
RequireDef=REQ1;REQ2
MaxUserSelect=7
NoStandardCrew=-2
ColorByMaterial=Granite
Explosive=3
DragImagePicture=4
TemporaryCrew=5
SmokeRate=88
NeededGfxMode=6
HideHUDBars=Energy|Breath
HideHUDElements=Portrait|Inventory
BurnTo=BURN
SolidMask=1,2,3,4,5
TopFace=6,7,8,9
Value=-9
ContactCalls=6
Exclusive=7
Rebuy=-2
CollectionLimit=-4
Vertices=-3
Scale=4294967291
NoGet=-8
Version=5,,2
VertexX=10,,30
Entrance=1,2,,4
"#,
        )
        .expect("complete reflection fields parse");

        check_eq! { parsed.require_defs => vec!["REQ1", "REQ2"] }
        check_eq! { parsed.max_user_select => 7 }
        check_eq! { parsed.no_standard_crew => -2 }
        check_eq! { parsed.color_by_material => "Granite" }
        check_eq! { parsed.explosive => 3 }
        check_eq! { parsed.drag_image_picture => 4 }
        check_eq! { parsed.temporary_crew => 5 }
        check_eq! { parsed.smoke_rate => 88 }
        check_eq! { parsed.needed_gfx_mode => 6 }
        check_eq! { parsed.hide_hud_bars => 5 }
        check_eq! { parsed.hide_hud_elements => 33 }
        check_eq! { parsed.burn_turn_to.as_deref() => Some("BURN") }
        check_eq! { parsed.reflected_ints.get("Value") => Some(&-9) }
        check_eq! { parsed.reflected_ints.get("ContactCalls") => Some(&6) }
        check_eq! { parsed.reflected_ints.get("Exclusive") => Some(&7) }
        check_eq! { parsed.reflected_ints.get("Rebuy") => Some(&-2) }
        check_eq! { parsed.reflected_ints.get("CollectionLimit") => Some(&-4) }
        check_eq! { parsed.reflected_ints.get("Vertices") => Some(&-3) }
        check_eq! { parsed.reflected_ints.get("Scale") => Some(&-5) }
        check_eq! { parsed.graphics_scale => 4_294_967_291 }
        check_eq! { parsed.reflected_ints.get("NoGet") => Some(&-8) }
        check_eq! { parsed.version => [5, 0, 2, 0, 0] }
        check_eq! { parsed.vertex_slots[0].x => 10 }
        check_eq! { parsed.vertex_slots[1].x => 0 }
        check_eq! { parsed.vertex_slots[2].x => 30 }
        check_eq! { parsed.entrance => Some(PictureRect {x: 1, y: 2, width: 0, height: 4,}) }
        check_eq! { parsed.solid_mask => Some(TargetRect {x: 1, y: 2, width: 3, height: 4, target_x: 5, target_y: 0,}) }
        check_eq! { parsed.top_face => Some(TargetRect {x: 6, y: 7, width: 8, height: 9, target_x: 0, target_y: 0,}) }

        let defaults = parse_def_core(b"[DefCore]\nid=DFLT\n").expect("defaults parse");
        check! { defaults.require_defs.is_empty() }
        check_eq! { defaults.max_user_select => 0 }
        check_eq! { defaults.no_standard_crew => 0 }
        check_eq! { defaults.color_by_material => "" }
        check_eq! { defaults.explosive => 0 }
        check_eq! { defaults.drag_image_picture => 0 }
        check_eq! { defaults.temporary_crew => 0 }
        check_eq! { defaults.smoke_rate => 100 }
        check_eq! { defaults.needed_gfx_mode => 0 }
        check_eq! { defaults.hide_hud_bars => 0 }
        check_eq! { defaults.hide_hud_elements => 0 }
    }

    // C4Def::CompileFunc maps Width/Height/Offset into Shape.Wdt/Hgt/x/y
    // (C4Def.cpp) — CR DefCores carry no combined Shape= key. The GoldRush
    // wagon COAC (Width=48 Height=40 Offset=-24,-20) needs this rect for
    // the NewObject bottom-growth adjust.
    // Facet= is a C4TargetRect: x,y,wdt,hgt plus DEFAULTED tx,ty — partial
    // lists are valid (C4TargetRect::CompileFunc mkDefaultAdapt,
    // C4Rect.cpp:80-86). Mage.c4d AimMagic uses the 5-value form
    // "0,328,24,20,-4".
    #[test]
    fn action_facet_accepts_partial_target_offsets_like_c4targetrect() {
        let five = parse_action_facet("0,328,24,20,-4").expect("5-value facet parses");
        check_eq! { (five.x, five.y, five.width, five.height, five.target_x, five.target_y) => (0, 328, 24, 20, -4, 0) }
        let six = parse_action_facet("0,260,16,24,0,-4").expect("6-value facet parses");
        check_eq! { (six.target_x, six.target_y) => (0, -4) }
        let four = parse_action_facet("0,0,16,20").expect("4-value facet parses");
        check_eq! { (four.target_x, four.target_y) => (0, 0) }
        let sparse = parse_action_facet("1,,3,4,,6").expect("empty slots default in place");
        check_eq! { (sparse.x, sparse.y, sparse.width, sparse.height, sparse.target_x, sparse.target_y) => (1, 0, 3, 4, 0, 6) }
        let malformed = parse_action_facet("1,bad,3,4").expect("bad slot defaults");
        check_eq! { (malformed.x, malformed.y, malformed.width, malformed.height) => (1, 0, 0, 0), "a failed primitive leaves the compiler cursor before later separators" }
        let trailing_junk = parse_action_facet("1junk,2,3,4,5,6").expect("numeric prefix parses");
        check_eq! { (trailing_junk.x, trailing_junk.y, trailing_junk.width, trailing_junk.height, trailing_junk.target_x, trailing_junk.target_y) => (1, 0, 0, 0, 0, 0), "a separator mismatch after a numeric prefix blocks later reads" }
    }

    #[test]
    fn defcore_width_height_offset_compose_the_shape_rect() {
        let core = parse_def_core(
            b"[DefCore]\nid=COAC\nName=Coach\nWidth=48\nHeight=40\nOffset=-24,-20\n",
        )
        .expect("core parses");
        let shape = core.shape.expect("shape synthesized");
        check_eq! { (shape.x, shape.y, shape.width, shape.height) => (-24, -20, 48, 40) }
    }
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[test]
    fn definition_graphics_names_preserve_legacy_bytes_and_c4maxname() {
        fn encoded_image(color: [u8; 4], format: image::ImageFormat) -> Vec<u8> {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(color));
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, format)
                .expect("encode graphics fixture");
            bytes.into_inner()
        }

        const CLIPPED_SUFFIX: &[u8] = b"123456789012345678901234567890";
        check_eq! { CLIPPED_SUFFIX.len() => C4_MAX_NAME_BYTES }

        let collision_bmp = [
            b"Graphics".as_slice(),
            CLIPPED_SUFFIX,
            b"Bmp.bmp".as_slice(),
        ]
        .concat();
        let first_png = [
            b"Graphics".as_slice(),
            CLIPPED_SUFFIX,
            b"First.png".as_slice(),
        ]
        .concat();
        let second_png = [
            b"Graphics".as_slice(),
            CLIPPED_SUFFIX,
            b"Second.png".as_slice(),
        ]
        .concat();
        let first_overlay = [
            b"Overlay".as_slice(),
            CLIPPED_SUFFIX,
            b"First.png".as_slice(),
        ]
        .concat();

        // A non-definition logical filename keeps the explicit insertion
        // order instead of applying MutableGroup's stock .c4d sort list.
        let mut packed = crate::MutableGroup::new("native-graphics.bin");
        packed
            .add_file_bytes_with_metadata(
                collision_bmp,
                b"suppressed invalid BMP".to_vec(),
                1,
                false,
            )
            .expect("add physically first BMP collision");
        packed
            .add_file_bytes_with_metadata(
                first_png,
                encoded_image([11, 22, 33, 255], image::ImageFormat::Png),
                1,
                false,
            )
            .expect("add first PNG collision");
        packed
            .add_file_bytes_with_metadata(
                second_png,
                encoded_image([44, 55, 66, 255], image::ImageFormat::Png),
                1,
                false,
            )
            .expect("add second PNG collision");
        packed
            .add_file_bytes_with_metadata(
                first_overlay,
                encoded_image([80, 90, 100, 255], image::ImageFormat::Png),
                1,
                false,
            )
            .expect("add full-suffix owner overlay");
        packed
            .add_file_bytes_with_metadata(
                b"Graphics\xfc.png".to_vec(),
                encoded_image([77, 88, 99, 255], image::ImageFormat::Png),
                1,
                false,
            )
            .expect("add native-byte named graphics");
        packed
            .add_file_bytes_with_metadata(
                b"Portrait\xf6.bmp".to_vec(),
                encoded_image([101, 102, 103, 255], image::ImageFormat::Bmp),
                1,
                false,
            )
            .expect("add native-byte packed portrait");
        let packed = Group::from_memory(
            PathBuf::from("native-graphics.c4d"),
            packed.pack().expect("pack native graphics group"),
        )
        .expect("open native graphics group");

        let (_, _, additional) =
            load_definition_graphics(&packed, true).expect("load exact packed graphics entries");
        check_eq! { additional.len() => 2 }

        let collision_name = clonk_script::c4_string_from_bytes(CLIPPED_SUFFIX);
        let collision = additional
            .get(&normalize_variant_key(&collision_name))
            .expect("truncated collision retained");
        check_eq! { clonk_script::c4_string_bytes(&collision.name) => CLIPPED_SUFFIX, "the suffix is clipped to 30 native bytes before lookup" }
        check_eq! { collision.image.pixels() => &[11, 22, 33, 255] }
        check_eq! { collision.color_by_owner_mask.as_ref().map(|mask| mask.pixels.as_slice()) => Some([80, 90, 100, 255].as_slice()), "the first PNG uses its untruncated suffix-matched overlay" }

        let legacy_name = clonk_script::c4_string_from_bytes(b"\xfc");
        let legacy = additional
            .get(&normalize_variant_key(&legacy_name))
            .expect("native-byte packed graphics retained");
        check_eq! { clonk_script::c4_string_bytes(&legacy.name) => b"\xfc" }
        check_eq! { legacy.image.pixels() => &[77, 88, 99, 255] }
        let uppercase_legacy_name = clonk_script::c4_string_from_bytes(b"\xdc");
        check! { additional.contains_key(&normalize_variant_key(&uppercase_legacy_name)), "C4 SEqualNoCase folds native umlaut pairs" }

        let packed_portraits =
            load_portrait_graphics(&packed, false).expect("load packed portraits");
        let packed_portrait = packed_portraits
            .iter()
            .find(|portrait| clonk_script::c4_string_bytes(&portrait.name) == b"\xf6")
            .expect("native-byte packed portrait retained");
        check_eq! { packed_portrait.image.pixels() => &[101, 102, 103, 255] }

        // A colliding PNG is loaded even though lookup keeps the first node.
        // This preserves the native fatal error from a corrupt losing PNG.
        let mut corrupt = crate::MutableGroup::new("corrupt-collision.bin");
        corrupt
            .add_file_bytes_with_metadata(
                [
                    b"Graphics".as_slice(),
                    CLIPPED_SUFFIX,
                    b"First.png".as_slice(),
                ]
                .concat(),
                encoded_image([1, 2, 3, 255], image::ImageFormat::Png),
                1,
                false,
            )
            .expect("add valid first PNG");
        corrupt
            .add_file_bytes_with_metadata(
                [
                    b"Graphics".as_slice(),
                    CLIPPED_SUFFIX,
                    b"Broken.png".as_slice(),
                ]
                .concat(),
                b"invalid PNG".to_vec(),
                1,
                false,
            )
            .expect("add invalid colliding PNG");
        let corrupt = Group::from_memory(
            PathBuf::from("corrupt-collision.c4d"),
            corrupt.pack().expect("pack corrupt collision group"),
        )
        .expect("open corrupt collision group");
        check! { matches!(load_definition_graphics(&corrupt, false), Err(DefinitionError::Graphics {..})) }

        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt as _;

            // Darwin rejects lone malformed UTF-8 path bytes. Other Unix
            // hosts exercise the literal legacy filename; macOS still checks
            // the same exact-entry directory path with a non-ASCII UTF-8 name.
            #[cfg(target_os = "macos")]
            const PORTRAIT_FILENAME: &[u8] = b"Portrait\xc3\xb6.bmp";
            #[cfg(target_os = "macos")]
            const PORTRAIT_SUFFIX: &[u8] = b"\xc3\xb6";
            #[cfg(not(target_os = "macos"))]
            const PORTRAIT_FILENAME: &[u8] = b"Portrait\xf6.bmp";
            #[cfg(not(target_os = "macos"))]
            const PORTRAIT_SUFFIX: &[u8] = b"\xf6";

            let directory = tempdir().expect("physical portrait directory");
            write_fixture! { directory.path().join(OsStr::from_bytes(PORTRAIT_FILENAME)) => encoded_image([121, 122, 123, 255], image::ImageFormat::Bmp), "write physical native-byte portrait" };
            let portraits = load_portrait_graphics(
                &Group::open(directory.path()).expect("open physical portrait group"),
                false,
            )
            .expect("load physical portrait");
            check_eq! { portraits.len() => 1 }
            check_eq! { clonk_script::c4_string_bytes(&portraits[0].name) => PORTRAIT_SUFFIX }
            check_eq! { portraits[0].image.pixels() => &[121, 122, 123, 255] }
        }
    }

    #[test]
    fn definition_images_blacken_transparent_rgb_before_owner_mask_generation() {
        definition_fixture_dir! { temp, def_dir => "Crew.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=CRWB\nColorByOwner=1\n", "DefCore" };
        let decoded_pixels = vec![0, 0, 255, 0, 17, 17, 17, 1];
        let save_surface = |name: &str| {
            image::RgbaImage::from_raw(2, 1, decoded_pixels.clone())
                .expect("rgba image")
                .save(def_dir.join(name))
                .expect("write image");
        };
        save_surface("Graphics.png");
        save_surface("Portrait1.png");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");
        let expected = [0, 0, 0, 0, 17, 17, 17, 1];

        check_eq! { definition.graphics_image.as_ref().expect("definition graphics").pixels() => expected }
        check! { definition.color_by_owner_mask.is_none(), "hidden blue RGB is cleared before C4's owner-color shade scan" }
        check_eq! { definition.portrait_image.as_ref().expect("plain portrait").pixels() => expected }
        check_eq! { definition.portrait_graphics_image.as_ref().expect("color-aware portrait").pixels() => expected }
        check! { definition.portrait_color_by_owner_mask.is_none() }
    }

    fn cpp_color_by_owner_gray(r: i32, g: i32, b: i32) -> Option<u8> {
        const HLSMAX: i32 = 255;
        const RGBMAX: i32 = 255;

        let c_max = r.max(g).max(b);
        let c_min = r.min(g).min(b);
        let l = ((c_max + c_min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
        if c_max == c_min {
            return None;
        }
        let s = if l <= HLSMAX / 2 {
            ((c_max - c_min) * HLSMAX + (c_max + c_min) / 2) / (c_max + c_min)
        } else {
            ((c_max - c_min) * HLSMAX + (2 * RGBMAX - c_max - c_min) / 2)
                / (2 * RGBMAX - c_max - c_min)
        };
        let rdelta = ((c_max - r) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
        let gdelta = ((c_max - g) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
        let bdelta = ((c_max - b) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
        let mut hue = if r == c_max {
            bdelta - gdelta
        } else if g == c_max {
            HLSMAX / 3 + rdelta - bdelta
        } else {
            2 * HLSMAX / 3 + gdelta - rdelta
        };
        if hue < 0 {
            hue += HLSMAX;
        }
        if hue > HLSMAX {
            hue -= HLSMAX;
        }
        ((145..=175).contains(&hue) && s > 100).then_some(b as u8)
    }

    #[test]
    fn auto_generated_color_by_owner_mask_matches_c4surface_color_sweep() {
        let surface_pixel = |r: u8, g: u8, b: u8| {
            u32::from(255_u8) << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        };
        check_eq! { detect_color_by_owner(surface_pixel(0, 0, 255)) => Some(255) }
        check_eq! { detect_color_by_owner(surface_pixel(255, 0, 0)) => None }

        for (rgb, expected) in [
            ([128, 128, 128], None),
            ([50, 50, 115], None),
            ([50, 50, 116], Some(116)),
            ([0, 152, 255], Some(255)),
            ([0, 157, 255], None),
            ([30, 0, 255], Some(255)),
            ([37, 0, 255], None),
            ([30, 30, 200], Some(200)),
        ] {
            check_eq! { detect_color_by_owner(surface_pixel(rgb[0], rgb[1], rgb[2])) => expected, "boundary color {rgb:?}" }
        }

        const CHANNELS: [u8; 24] = [
            0, 1, 2, 15, 16, 31, 32, 41, 42, 43, 63, 85, 100, 101, 127, 128, 145, 170, 175, 191,
            223, 240, 254, 255,
        ];
        let side = CHANNELS.len() as u32;
        let width = side * side;
        let mut image = image::RgbaImage::new(width, side);
        let mut expected = vec![None; (width * side) as usize];
        for (r_index, &r) in CHANNELS.iter().enumerate() {
            for (g_index, &g) in CHANNELS.iter().enumerate() {
                for (b_index, &b) in CHANNELS.iter().enumerate() {
                    let x = r_index as u32 * side + g_index as u32;
                    let y = b_index as u32;
                    image.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                    expected[(y * width + x) as usize] =
                        cpp_color_by_owner_gray(i32::from(r), i32::from(g), i32::from(b));
                }
            }
        }

        let mask = generate_color_by_owner_mask(&mut image).expect("sweep contains blue shades");
        for (index, expected) in expected.into_iter().enumerate() {
            check_eq! { mask.pixels[index] => expected.unwrap_or(0), "sample index {index}" }
        }
    }

    // Overlay.png is the ClrByOwner surface itself: C4DefGraphics::LoadGraphics
    // keeps it as BitmapClr with the base as pMainSfc (C4DefGraphics.cpp:74-94,
    // C4Surface::SetAsClrByOwnerOf, C4Surface.cpp:320-331), so drawing blits
    // the overlay pixel modulated by the owner color OVER the base using the
    // OVERLAY's alpha. The Mage body lives only in Overlay.png (base cells are
    // transparent apart from the staff), so both surfaces must remain intact.
    #[test]
    fn explicit_owner_overlay_retains_rgba_and_leaves_base_unchanged() {
        let mut base = image::RgbaImage::from_pixel(4, 1, image::Rgba([100, 64, 35, 0]));
        base.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        base.put_pixel(2, 0, image::Rgba([80, 50, 20, 255]));
        base.put_pixel(3, 0, image::Rgba([11, 22, 33, 255]));
        let original_base = base.clone();
        let mut overlay = image::RgbaImage::from_pixel(4, 1, image::Rgba([100, 100, 100, 0]));
        overlay.put_pixel(0, 0, image::Rgba([136, 136, 136, 255]));
        overlay.put_pixel(1, 0, image::Rgba([128, 128, 128, 128]));
        overlay.put_pixel(2, 0, image::Rgba([0, 0, 255, 255]));
        overlay.put_pixel(3, 0, image::Rgba([0, 0, 0, 255]));

        let mask = extract_mask_from_overlay(&overlay, &base).expect("mask extracted");

        // The base remains byte-exact; Overlay.png is a second surface, not a
        // scalar mask baked into the first one.
        check_eq! { base => original_base }
        check_eq! { mask.pixels => overlay.as_raw().to_vec() }
    }

    #[test]
    fn picture_uses_decoded_base_graphics_and_corrupt_graphics_is_typed() {
        definition_fixture_dir! { temp, def_dir => "Picture.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=PICT\nPicture=0,0,1,1\n", "DefCore" };
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("base graphics");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([200, 210, 220, 255]))
            .save(def_dir.join("Graphics32.png"))
            .expect("additional graphics");
        write_fixture! { def_dir.join("Graphics32.bmp") => b"not a bitmap", "ignored losing additional bitmap" };
        write_fixture! { def_dir.join("GraphicsIgnored.jpg") => b"not a jpeg", "ignored non-native graphics format" };
        fs::create_dir(def_dir.join("Graphics.c4g")).expect("nested graphics directory");
        write_fixture! { def_dir.join("Graphics.c4g/GraphicsNested.png") => b"not a png", "ignored nested graphics" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("valid graphics load");
        let picture = definition.picture_image.expect("base picture crop");
        check_eq! { (picture.width(), picture.height()) => (1, 1) }
        check_eq! { picture.pixels() => &[10, 20, 30, 255] }

        write_fixture! { def_dir.join("Graphics32.png") => b"not a png", "corrupt recognized additional Graphics" };
        check! { matches!(Definition::load(&group), Err(DefinitionError::Graphics {path, reason}) if path == Path::new("Graphics32.png") && !reason.is_empty()) }

        let blank_dir = temp.path().join("Blank.c4d");
        fs::create_dir(&blank_dir).expect("blank definition directory");
        write_fixture! { blank_dir.join("DefCore.txt") => b"[DefCore]\nid=BLNK\n", "blank DefCore" };
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]))
            .save(blank_dir.join("Graphics.png"))
            .expect("transparent base graphics");
        let blank = Definition::load(&Group::open(&blank_dir).expect("open blank definition"))
            .expect("zero-sized effective picture remains a valid loaded definition");
        check! { blank.picture_image.is_none() }

        let transparent_dir = temp.path().join("Transparent.c4d");
        fs::create_dir(&transparent_dir).expect("transparent definition directory");
        write_fixture! { transparent_dir.join("DefCore.txt") => b"[DefCore]\nid=TRNS\nPicture=0,0,1,1\n", "transparent DefCore" };
        image::RgbaImage::from_pixel(1, 1, image::Rgba([99, 88, 77, 0]))
            .save(transparent_dir.join("Graphics.png"))
            .expect("transparent base graphics");
        let transparent =
            Definition::load(&Group::open(&transparent_dir).expect("open transparent definition"))
                .expect("transparent picture remains a valid loaded definition");
        let transparent_picture = transparent.picture_image.expect("nonzero picture crop");
        check_eq! { transparent_picture.pixels() => &[0, 0, 0, 0] }
    }

    #[test]
    fn definition_picture_carries_its_cropped_color_by_owner_mask() {
        // C4Def::Picture2Facet takes PictureRect from the definition's
        // ColorByOwner-aware Graphics.GetBitmap(color) surface
        // (src/C4Def.cpp:1374-1378). LoadGraphics keeps Overlay.png as the
        // owner-color surface (src/C4DefGraphics.cpp:73-98), so the picture
        // crop must retain the matching mask instead of freezing its raw
        // Graphics.png colors.
        definition_fixture_dir! { temp, def_dir => "Mage.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=MAGE\nColorByOwner=1\nPicture=1,0,1,1\n", "DefCore" };

        let base = image::RgbaImage::from_pixel(3, 1, image::Rgba([0, 0, 0, 0]));
        base.save(def_dir.join("Graphics.png")).expect("base png");
        let mut overlay = image::RgbaImage::from_pixel(3, 1, image::Rgba([0, 0, 0, 0]));
        overlay.put_pixel(1, 0, image::Rgba([136, 136, 136, 255]));
        overlay
            .save(def_dir.join("Overlay.png"))
            .expect("overlay png");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");
        let picture = definition.picture_image.expect("picture crop");
        check_eq! { (picture.width(), picture.height()) => (1, 1) }
        check_eq! { picture.pixels() => &[0, 0, 0, 0], "the separately retained overlay must not be baked into the base picture" }
        let mask = definition
            .picture_color_by_owner_mask
            .expect("picture must retain owner-color mask");
        check_eq! { (mask.width, mask.height) => (1, 1) }
        check_eq! { mask.pixels => vec![136, 136, 136, 255] }
    }

    #[test]
    fn scaled_definition_picture_crops_the_scaled_source_rect() {
        // C4Def::Picture2Facet composes the phase offset in game units and then
        // scales the whole rect into bitmap space:
        // `C4Rect{PictureRect.x + xPhase * PictureRect.Wdt, ...}.Scaled(Scale)`
        // (src/C4Def.cpp:1341), where C4Rect::Scaled truncates each component as
        // `int32_t(float(val) * scale)` (src/C4Rect.cpp:37-44). C4Def::Draw
        // reaches the identical source region by passing Scale down to the blit:
        // `float(X + Wdt * iPhaseX) * scale, ..., float(Wdt) * scale`
        // (src/C4Facet.cpp:137). A Scale=200 definition therefore takes
        // Picture=0,0,1,1 from the bitmap rect (0,0,2,2), not the raw 1x1 rect.
        definition_fixture_dir! { temp, def_dir => "Scaled.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=SCAL\nScale=200\nPicture=0,0,1,1\n", "DefCore" };

        let mut graphics = image::RgbaImage::new(2, 2);
        graphics.put_pixel(0, 0, image::Rgba([1, 0, 0, 255]));
        graphics.put_pixel(1, 0, image::Rgba([2, 0, 0, 255]));
        graphics.put_pixel(0, 1, image::Rgba([3, 0, 0, 255]));
        graphics.put_pixel(1, 1, image::Rgba([4, 0, 0, 255]));
        graphics
            .save(def_dir.join("Graphics.png"))
            .expect("scaled graphics");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load scaled definition");
        let picture = definition.picture_image.expect("scaled picture crop");
        check_eq! { (picture.width(), picture.height()) => (2, 2) }
        check_eq! { picture.pixels() => &[1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255] }
    }

    #[test]
    fn def_core_load_defaults_missing_or_zero_picture_to_shape() {
        let temp = tempdir().expect("tempdir");
        let expected_default = PictureRect {
            x: 0,
            y: 0,
            width: 42,
            height: 48,
        };
        for (name, picture, expected) in [
            ("missing", "", expected_default),
            ("zero_width", "Picture=9,8,0,5\n", expected_default),
            ("zero_height", "Picture=9,8,5,0\n", expected_default),
            (
                "explicit",
                "Picture=3,4,5,6\n",
                PictureRect {
                    x: 3,
                    y: 4,
                    width: 5,
                    height: 6,
                },
            ),
        ] {
            let directory = temp.path().join(format!("{name}.c4d"));
            fs::create_dir(&directory).expect("definition directory");
            write_fixture! { directory.join("DefCore.txt") => format!("[DefCore]\nid=PICT\nWidth=42\nHeight=48\nOffset=-21,-24\n{picture}"), "DefCore" };
            let group = Group::open(&directory).expect("open definition");
            let core = DefCore::load(&group).expect("load DefCore");
            check_eq! { core.picture => Some(expected), "{name}" }
        }
    }

    #[test]
    fn defaulted_picture_crops_graphics_and_owner_mask_to_shape() {
        for (name, picture) in [("missing", ""), ("zero", "Picture=0,0,0,0\n")] {
            let temp = tempdir().expect("tempdir");
            let directory = temp.path().join(format!("{name}.c4d"));
            fs::create_dir(&directory).expect("definition directory");
            write_fixture! { directory.join("DefCore.txt") => format!("[DefCore]\nid=PICT\nWidth=2\nHeight=1\nOffset=-7,-8\nColorByOwner=1\n{picture}"), "DefCore" };
            image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 0]))
                .save(directory.join("Graphics.png"))
                .expect("base png");
            image::RgbaImage::from_pixel(4, 2, image::Rgba([136, 136, 136, 255]))
                .save(directory.join("Overlay.png"))
                .expect("overlay png");

            let group = Group::open(&directory).expect("open definition");
            let definition = Definition::load(&group).expect("load definition");
            check_eq! { definition.core.picture => Some(PictureRect {x: 0, y: 0, width: 2, height: 1,}), "{name}" }
            let image = definition.picture_image.expect("picture crop");
            check_eq! { (image.width(), image.height()) => (2, 1), "{name}" }
            let mask = definition
                .picture_color_by_owner_mask
                .expect("picture owner mask");
            check_eq! { (mask.width, mask.height) => (2, 1), "{name}" }
            check_eq! { mask.pixels => [136, 136, 136, 255].repeat(2), "{name}" }
        }
    }

    #[test]
    fn shipped_knights_tent_picture_is_the_top_left_shape_cell() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Knights.c4d/Camp.c4d/Tent.c4d");
        let group = Group::open(&directory).expect("open shipped Tent.c4d");
        let definition = Definition::load(&group).expect("load shipped Tent.c4d");

        check_eq! { definition.core.picture => Some(PictureRect {x: 0, y: 0, width: 42, height: 48,}) }
        let picture = definition.picture_image.expect("Tent picture crop");
        check_eq! { (picture.width(), picture.height()) => (42, 48) }
        let mask = definition
            .picture_color_by_owner_mask
            .expect("Tent picture owner mask");
        check_eq! { (mask.width, mask.height) => (42, 48) }
    }

    #[test]
    fn definition_portrait_carries_its_color_by_owner_mask() {
        // C4DefGraphics::LoadAllGraphics maps Portrait1.png to Overlay1.png
        // and loads both with ColorByOwner enabled (C4DefGraphics.cpp:166-205,
        // C4Def.cpp:1250-1264). DrawTextSpecImage later applies the requested
        // portrait color through GetBitmap(dwClr) (C4Game.cpp:4310-4324).
        definition_fixture_dir! { temp, def_dir => "Sorcerer.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=SCLK\nColorByOwner=1\n", "DefCore" };

        let base = image::RgbaImage::from_pixel(1, 1, image::Rgba([80, 50, 20, 0]));
        base.save(def_dir.join("Portrait1.png"))
            .expect("portrait png");
        let overlay = image::RgbaImage::from_pixel(1, 1, image::Rgba([136, 136, 136, 255]));
        overlay
            .save(def_dir.join("Overlay1.png"))
            .expect("portrait overlay png");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 0]))
            .save(def_dir.join("PortraitCaptain1.png"))
            .expect("named portrait png");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([64, 64, 64, 255]))
            .save(def_dir.join("OverlayCaptain1.png"))
            .expect("named portrait overlay png");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");
        let portrait = definition
            .portrait_graphics_image
            .expect("color-aware portrait image");
        check_eq! { portrait.pixels() => &[0, 0, 0, 0] }
        let mask = definition
            .portrait_color_by_owner_mask
            .expect("portrait must retain owner-color mask");
        check_eq! { (mask.width, mask.height) => (1, 1) }
        check_eq! { mask.pixels => vec![136, 136, 136, 255] }
        let named = definition
            .portrait_graphics
            .iter()
            .find(|portrait| portrait.name.eq_ignore_ascii_case("captain1"))
            .expect("named portrait retained");
        check_eq! { named.name => "Captain1" }
        check_eq! { (named.image.width(), named.image.height()) => (2, 1) }
        check_eq! { named.color_by_owner_mask.as_ref().map(|mask| mask.pixels.as_slice()) => Some([64, 64, 64, 255, 64, 64, 64, 255].as_slice()) }
    }

    #[test]
    fn shipped_clonk_portrait_auto_generates_its_owner_color_mask() {
        let directory = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/Objects.c4d/Crew.c4d/Clonk.c4d"
        ));
        check! { directory.is_dir(), "the initialized official content submodule must provide {}", directory.display() }

        let definition = Definition::load(&Group::open(directory).expect("open Clonk definition"))
            .expect("load Clonk definition");
        let primary = definition
            .portrait_color_by_owner_mask
            .as_ref()
            .expect("Portrait1 blue shades generate an owner-color mask");
        check_eq! { (primary.width, primary.height) => (150, 150) }
        check! { primary.pixels.iter().any(|value| *value != 0) }

        let retained = definition
            .portrait_graphics
            .iter()
            .find(|portrait| portrait.name == "1")
            .expect("Portrait1 retained in the full portrait set");
        check! { retained.color_by_owner_mask.as_ref().is_some_and(|mask| mask.pixels.iter().any(|value| *value != 0)) }
    }

    #[test]
    fn shipped_knight_shield_uses_its_suffix_matched_overlay() {
        let directory = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/Knights.c4d/Crew.c4d/Knight.c4d"
        ));
        check! { directory.is_dir(), "the initialized official content submodule must provide {}", directory.display() }

        let group = Group::open(directory).expect("open Knight definition");
        let definition = Definition::load(&group).expect("load Knight definition");
        let shield = definition
            .additional_graphics
            .get("shield")
            .expect("GraphicsShield retained");

        let mut expected_image = image::load_from_memory(
            &group
                .read_file("GraphicsShield.png")
                .expect("read shield graphics"),
        )
        .expect("decode shield graphics")
        .into_rgba8();
        let mut overlay = image::load_from_memory(
            &group
                .read_file("OverlayShield.png")
                .expect("read shield overlay"),
        )
        .expect("decode shield overlay")
        .into_rgba8();
        blacken_fully_transparent_rgba(expected_image.as_mut());
        blacken_fully_transparent_rgba(overlay.as_mut());
        let expected_mask = extract_mask_from_overlay(&overlay, &expected_image)
            .expect("shield overlay contains an owner-color mask");

        let actual_mask = shield
            .color_by_owner_mask
            .as_ref()
            .expect("suffix-matched shield overlay loaded");
        check_eq! { (actual_mask.width, actual_mask.height) => (expected_mask.width, expected_mask.height) }
        check_eq! { actual_mask.pixels => expected_mask.pixels }
        check_eq! { shield.image.pixels() => expected_image.as_raw() }
    }

    fn indexed_definition_bmp(indices: Vec<u8>) -> Vec<u8> {
        let width = indices.len() as u32;
        let bitmap = crate::bitmap::IndexedBitmap {
            width,
            height: 1,
            indices,
        };
        let mut misleading_file_palette = [[0_u8; 3]; 256];
        misleading_file_palette[0] = [159, 169, 251];
        misleading_file_palette[1] = [200, 4, 8];
        misleading_file_palette[191] = [200, 0, 0];
        misleading_file_palette[255] = [255, 255, 255];
        bitmap
            .encode_with_palette(&misleading_file_palette)
            .expect("indexed definition BMP encodes")
    }

    #[test]
    fn indexed_definition_bmps_use_game_palette_for_graphics_picture_and_ranks() {
        definition_fixture_dir! { temp, def_dir => "Indexed.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=IBMP\nPicture=0,0,4,1\n", "DefCore" };
        let bmp = indexed_definition_bmp(vec![0, 1, 191, 255]);
        write_fixture! { def_dir.join("Graphics.bmp") => &bmp, "Graphics.bmp" };
        write_fixture! { def_dir.join("Rank.bmp") => &bmp, "Rank.bmp" };

        let definition = Definition::load(&Group::open(&def_dir).expect("open definition"))
            .expect("load indexed definition");
        let expected = [
            [0, 0, 0, 0],
            [52, 52, 52, 255],
            [0, 0, 255, 128],
            [0, 0, 0, 255],
        ]
        .concat();
        check_eq! { definition.graphics_image.as_ref().expect("graphics image").pixels() => expected, "Graphics.bmp indices use expanded C4.PAL colors and AlphaPalette" }
        check_eq! { definition.picture_image.as_ref().expect("picture image").pixels() => expected, "the cropped definition picture uses the same decoded palette" }
        check_eq! { definition.rank_symbols_image.as_ref().expect("rank strip").pixels() => expected, "Rank.bmp uses the same non-owning palette path" }
        check_eq! { definition.rank_symbol_count => Some(4) }
    }

    #[test]
    fn indexed_game_palette_blue_remains_half_alpha_color_by_owner() {
        definition_fixture_dir! { temp, def_dir => "IndexedOwner.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=IOWN\nColorByOwner=1\n", "DefCore" };
        write_fixture! { def_dir.join("Graphics.bmp") => indexed_definition_bmp(vec![191]), "Graphics.bmp" };

        let definition = Definition::load(&Group::open(&def_dir).expect("open definition"))
            .expect("load indexed owner-color definition");
        check_eq! { definition.graphics_image.as_ref().expect("graphics image").pixels() => &[255, 255, 255, 128], "the owner-color sweep preserves index 191's half alpha" }
        check_eq! { definition.color_by_owner_mask.as_ref().expect("blue index generates owner-color mask").pixels => vec![255] }
    }

    #[test]
    fn truecolor_definition_bmp_keeps_file_rgb() {
        definition_fixture_dir! { temp, def_dir => "Truecolor.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=TRGB\n", "DefCore" };
        let path = def_dir.join("Graphics.bmp");
        image::RgbImage::from_raw(2, 1, vec![7, 23, 211, 240, 17, 99])
            .expect("RGB image")
            .save_with_format(&path, image::ImageFormat::Bmp)
            .expect("24-bit Graphics.bmp");
        let encoded = fs::read(&path).expect("read Graphics.bmp");
        check_eq! { u16::from_le_bytes([encoded[28], encoded[29]]) => 24 }

        let definition = Definition::load(&Group::open(&def_dir).expect("open definition"))
            .expect("load truecolor definition");
        check_eq! { definition.graphics_image.as_ref().expect("graphics image").pixels() => &[7, 23, 211, 255, 240, 17, 99, 255], "24-bit BMPs remain on the generic truecolor decoder" }
    }

    #[test]
    fn shipped_lastwill_dialog_game_palette_index_zero_is_fully_transparent() {
        let directory = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/Missions.c4f/LastWill.c4s/Dlg.c4d"
        ));
        check! { directory.is_dir(), "the initialized official content submodule must provide {}", directory.display() }

        let definition = Definition::load(&Group::open(directory).expect("open Dlg definition"))
            .expect("load Dlg definition");
        let image = definition.graphics_image.as_ref().expect("graphics image");
        check_eq! { (image.width(), image.height()) => (16, 20) }
        check! { image.pixels().chunks_exact(4).all(|pixel| pixel == [0, 0, 0, 0]), "all-index-zero Graphics.bmp must be invisible, not file-palette pink" }
    }

    #[test]
    fn missing_variant_overlay_and_bmp_portrait_auto_generate_masks() {
        definition_fixture_dir! { temp, def_dir => "AutoMasks.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=AUTO\nColorByOwner=1\n", "DefCore" };

        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("base graphics");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([32, 32, 32, 255]))
            .save(def_dir.join("Overlay.png"))
            .expect("base overlay");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 136, 255]))
            .save(def_dir.join("GraphicsAuto.png"))
            .expect("named graphics");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 144, 255]))
            .save(def_dir.join("Portrait1.png"))
            .expect("PNG portrait");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 160, 255]))
            .save(def_dir.join("PortraitLegacy.bmp"))
            .expect("BMP portrait");

        let group = Group::open(&def_dir).expect("open definition");
        let mut expected_auto = image::load_from_memory(
            &group
                .read_file("GraphicsAuto.png")
                .expect("read named graphics"),
        )
        .expect("decode named graphics")
        .into_rgba8();
        let expected_auto_mask = generate_color_by_owner_mask(&mut expected_auto)
            .expect("named graphics contain an auto-mask shade");
        let mut expected_primary_portrait =
            image::load_from_memory(&group.read_file("Portrait1.png").expect("read PNG portrait"))
                .expect("decode PNG portrait")
                .into_rgba8();
        let expected_primary_portrait_mask =
            generate_color_by_owner_mask(&mut expected_primary_portrait)
                .expect("PNG portrait contains an auto-mask shade");
        let mut expected_portrait = image::load_from_memory(
            &group
                .read_file("PortraitLegacy.bmp")
                .expect("read BMP portrait"),
        )
        .expect("decode BMP portrait")
        .into_rgba8();
        let expected_portrait_mask = generate_color_by_owner_mask(&mut expected_portrait)
            .expect("BMP portrait contains an auto-mask shade");

        let definition = Definition::load(&group).expect("load definition");
        let named = definition
            .additional_graphics
            .get("auto")
            .expect("named graphics retained");
        let named_mask = named
            .color_by_owner_mask
            .as_ref()
            .expect("missing OverlayAuto.png triggers auto-generation");
        check_eq! { named_mask.pixels => expected_auto_mask.pixels }
        check_eq! { named.image.pixels() => expected_auto.as_raw() }
        check_ne! { named_mask.pixels => vec![32] }

        let primary_portrait_mask = definition
            .portrait_color_by_owner_mask
            .as_ref()
            .expect("missing Overlay1.png triggers portrait auto-generation");
        check_eq! { primary_portrait_mask.pixels => expected_primary_portrait_mask.pixels }
        check_eq! { definition.portrait_graphics_image.as_ref().expect("PNG portrait retained").pixels() => expected_primary_portrait.as_raw() }
        check_ne! { primary_portrait_mask.pixels => vec![32] }

        let portrait = definition
            .portrait_graphics
            .iter()
            .find(|portrait| portrait.name == "Legacy")
            .expect("BMP portrait retained");
        let portrait_mask = portrait
            .color_by_owner_mask
            .as_ref()
            .expect("BMP portrait auto-generates without consulting Overlay.png");
        check_eq! { portrait_mask.pixels => expected_portrait_mask.pixels }
        check_eq! { portrait.image.pixels() => expected_portrait.as_raw() }
        check_ne! { portrait_mask.pixels => vec![32] }
    }

    #[test]
    fn invalid_exact_owner_overlay_rejects_definition() {
        definition_fixture_dir! { temp, def_dir => "BadOverlay.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=BADG\nColorByOwner=1\n", "DefCore" };
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("base graphics");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([32, 32, 32, 255]))
            .save(def_dir.join("Overlay.png"))
            .expect("base overlay");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([136, 0, 0, 255]))
            .save(def_dir.join("GraphicsBad.png"))
            .expect("named graphics");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([64, 64, 64, 255]))
            .save(def_dir.join("OverlayBad.png"))
            .expect("wrong-size named overlay");

        let group = Group::open(&def_dir).expect("open definition");
        check! { matches!(Definition::load(&group), Err(DefinitionError::ColorByOwnerOverlay {path, reason}) if path == Path::new("OverlayBad.png") && reason.contains("does not match")) }

        image::RgbaImage::from_pixel(1, 1, image::Rgba([64, 64, 64, 255]))
            .save_with_format(def_dir.join("OverlayBad.png"), image::ImageFormat::Bmp)
            .expect("write BMP bytes under a PNG overlay name");
        check! { matches!(Definition::load(&group), Err(DefinitionError::ColorByOwnerOverlay {path,..}) if path == Path::new("OverlayBad.png")) }
    }

    #[test]
    fn custom_rank_symbol_count_uses_localized_rank_file_priority() {
        // C4Def loads Rank{}.txt|Rank.txt with the active language sequence,
        // then reserves one trailing strip cell for each leading-'*' rank
        // extension (C4Def.cpp:659-706; C4RankSystem.cpp:96-180).
        definition_fixture_dir! { temp, def_dir => "Ranked.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=RANK\n", "DefCore" };
        image::RgbaImage::from_pixel(5, 1, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.png"))
            .expect("rank strip");
        write_fixture! { def_dir.join("RankUS.txt") => b"Recruit\r\n*First %s\r\n", "US ranks" };
        write_fixture! { def_dir.join("RankDE.txt") => b"Rekrut\r\n*Erster %s\r\n*Zweiter %s\r\n", "DE ranks" };
        write_fixture! { def_dir.join("Rank.txt") => b"Fallback\n*One %s\n*Two %s\n*Three %s\n", "fallback ranks" };

        let group = Group::open(&def_dir).expect("open definition");
        let us = Definition::load_with_languages(&group, &["US", "DE"])
            .expect("load US-priority definition");
        check_eq! { us.rank_symbol_count => Some(4) }
        check_eq! { us.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Recruit".to_string(), "First Recruit".to_string()]) }

        let de = Definition::load_with_languages(&group, &["DE", "US"])
            .expect("load DE-priority definition");
        check_eq! { de.rank_symbol_count => Some(3) }
        check_eq! { de.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Rekrut".to_string(), "Erster Rekrut".to_string(), "Zweiter Rekrut".to_string(),]) }

        let fallback =
            Definition::load_with_languages(&group, &["FR"]).expect("load fallback definition");
        check_eq! { fallback.rank_symbol_count => Some(2) }
        check_eq! { fallback.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Fallback".to_string(), "One Fallback".to_string(), "Two Fallback".to_string(), "Three Fallback".to_string(),]) }
    }

    #[test]
    fn language_pack_rank_names_require_a_local_rank_marker() {
        // C4Def first probes its own group with FindEntry("Rank*.txt"). Only
        // after that succeeds does RankSystem::LoadEx search language packs.
        let temp = tempdir().expect("tempdir");
        let content = temp.path().join("content");
        let def_dir = content.join("Ranked.c4d");
        fs::create_dir_all(&def_dir).expect("definition directory");
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=RANK\n", "DefCore" };

        let language_container = temp.path().join("Language.c4g");
        let pack_def = language_container.join("Pack.c4g/Ranked.c4d");
        fs::create_dir_all(&pack_def).expect("pack definition directory");
        write_fixture! { pack_def.join("RankUS.txt") => b"Packed recruit\r\n", "pack rank names" };

        let packs = crate::LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            std::slice::from_ref(&content),
        );
        let group = Group::open(&def_dir).expect("open definition");
        let components = packs.component_groups(&group, None, None);

        let without_marker =
            Definition::load_with_languages_and_components(&group, &["US", "DE"], &components)
                .expect("load definition without local marker");
        check_eq! { without_marker.rank_names => None }

        write_fixture! { def_dir.join("RankDE.txt") => b"Lokaler Marker\r\n", "local rank marker" };
        let with_marker =
            Definition::load_with_languages_and_components(&group, &["US", "DE"], &components)
                .expect("load definition with local marker");
        check_eq! { with_marker.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Packed recruit".to_string()]) }
    }

    #[test]
    fn custom_rank_names_expand_extensions_in_cpp_order() {
        definition_fixture_dir! { temp, def_dir => "ExpandedRanks.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=EXPR\n", "DefCore" };
        write_fixture! { def_dir.join("RankUS.txt") => b"# comment\r\n*First %s\r\nBase=500\r\nRecruit\r\nIgnored=setting\r\nVeteran\r\n*100%% %s\r\nUnterminated", "rank names" };

        let definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("load definition");
        assert_eq!(
            definition
                .rank_names
                .as_ref()
                .map(RankNameTable::resolved_names),
            Some(vec![
                "Recruit".to_string(),
                "Veteran".to_string(),
                "First Recruit".to_string(),
                "First Veteran".to_string(),
                "100% Recruit".to_string(),
                "100% Veteran".to_string(),
            ])
        );
        check_eq! { definition.rank_base => Some(500) }
    }

    #[test]
    fn custom_rank_extensions_apply_printf_width_and_precision_like_cpp() {
        definition_fixture_dir! { temp, def_dir => "FormattedRanks.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=FMTR\n", "DefCore" };
        write_fixture! { def_dir.join("RankUS.txt") => b"Recruit\r\n\
        *Right|%10s|\r\n\
        *Left|%-10s|\r\n\
        *Precision|%.4s|\r\n\
        *Combined|%8.4s|\r\n\
        *LeftCombined|%-8.4s|\r\n\
        *Flags|%+ #010.4s|\r\n\
        *Escaped|100%% %s|\r\n\
        *Empty|%.s|\r\n\
        *Position|%1$8.4s/%1$s|\r\n\
        *Length|%1$hhs/%1$Ls|\r\n\
        *Literal|plain%%|\r\n", "formatted rank names" };

        let definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("load formatted ranks");
        assert_eq!(
            definition
                .rank_names
                .as_ref()
                .map(RankNameTable::resolved_names),
            Some(vec![
                "Recruit".to_string(),
                "Right|   Recruit|".to_string(),
                "Left|Recruit   |".to_string(),
                "Precision|Recr|".to_string(),
                "Combined|    Recr|".to_string(),
                "LeftCombined|Recr    |".to_string(),
                "Flags|      Recr|".to_string(),
                "Escaped|100% Recruit|".to_string(),
                "Empty||".to_string(),
                "Position|    Recr/Recruit|".to_string(),
                "Length|Recruit/Recruit|".to_string(),
                "Literal|plain%|".to_string(),
            ])
        );

        check_eq! { format_rank_extension(b"%.2s", "éclair".as_bytes()).expect("UTF-8 precision") => "é".as_bytes() }
        check_eq! { format_rank_extension(b"%4s", "界".as_bytes()).expect("wide UTF-8 padding") => "  界".as_bytes() }
        check_eq! { format_rank_extension(b"%3s", b"\xfc").expect("legacy-byte padding") => b"  \xfc" }

        let pointer = format_rank_extension(b"%p", b"Recruit").expect("C-string pointer format");
        check! { pointer.starts_with(b"0x") }
        check! { pointer[2..].iter().all(u8::is_ascii_hexdigit) }

        for (format, expected_reason) in [
            (b"%".as_slice(), "invalid format specifier"),
            (b"%d".as_slice(), "invalid format specifier"),
            (b"%s/%s".as_slice(), "argument not found"),
            (b"%2$s".as_slice(), "argument not found"),
            (
                b"%1$s/%s".as_slice(),
                "cannot switch from manual to automatic argument indexing",
            ),
            (b"%*s".as_slice(), "width is not integer"),
            (b"%.*s".as_slice(), "precision is not integer"),
            (b"%00000000001s".as_slice(), "number is too big"),
            (b"%00000000001$s".as_slice(), "argument not found"),
            (b"%0$.*s".as_slice(), "argument not found"),
        ] {
            check_eq! { format_rank_extension(format, b"Recruit") => Err(expected_reason), "format {}", String::from_utf8_lossy(format) }
        }
        check_eq! { format_rank_extension(b"%.00000000001s", b"Recruit") => Ok(Vec::new()), "fmt maps an oversized literal precision to its zero sentinel" }

        write_fixture! { def_dir.join("RankUS.txt") => b"Recruit\r\n*Valid %4.2s\r\n*Wrong %d\r\n", "invalid rank format" };
        let invalid_definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("reopen definition"),
            &["US"],
        )
        .expect("native stores malformed extensions without validating them");
        let invalid_table = invalid_definition
            .rank_names
            .as_ref()
            .expect("rank table remains installed");
        check_eq! { invalid_table.get(0).as_deref() => Some("Recruit") }
        check_eq! { invalid_table.get(1).as_deref() => Some("Valid   Re") }
        check_eq! { invalid_table.try_rank_name(usize::MAX, false).expect("an undefined non-fallback rank does not parse extensions") => None }
        let expected_error = RankExtensionFormatError {
            format: "Wrong %d".to_string(),
            reason: "invalid format specifier",
        };
        check_eq! { invalid_table.try_rank_name(2, false).expect_err("requesting the malformed extension must fail") => expected_error }
        check_eq! { invalid_table.try_rank_name(usize::MAX, true).expect_err("fallback clamps to and evaluates the malformed final extension") => expected_error }
        check! { std::panic::catch_unwind(|| invalid_table.get(2)).is_err(), "the normal rank lookup preserves native's uncaught error boundary" }

        write_fixture! { def_dir.join("RankUS.txt") => b"Recruit\r\n*%p\r\n", "pointer rank format" };
        let pointer_definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("reopen pointer definition"),
            &["US"],
        )
        .expect("load pointer ranks");
        let pointer_table = pointer_definition.rank_names.expect("pointer rank table");
        let pointer_clone = pointer_table.clone();
        let pointer_name = pointer_table.get(1).expect("pointer rank").into_owned();
        check! { pointer_name.starts_with("0x") }
        check_eq! { pointer_clone.get(1).as_deref() => Some(pointer_name.as_str()), "Arc-backed rank bytes keep `%p` stable across engine table clones" }
    }

    #[test]
    fn custom_rank_base_matches_cpp_setting_parsing() {
        definition_fixture_dir! { temp, def_dir => "RankBase.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=RBAS\n", "DefCore" };

        write_fixture! { def_dir.join("RankUS.txt") => b"Base=  +500suffix\r\nBase=invalid\r\nRecruit\r\nBase=250", "custom rank base" };
        let parsed = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("load definition");
        check_eq! { parsed.rank_base => Some(500), "scanf accepts a signed numeric prefix, malformed settings keep the prior value, and an unterminated line is ignored" }

        write_fixture! { def_dir.join("RankUS.txt") => b"Base=500\nBase=0trailing\nRecruit\n", "zero rank base" };
        let zero = Definition::load_with_languages(
            &Group::open(&def_dir).expect("reopen definition"),
            &["US"],
        )
        .expect("reload definition");
        check_eq! { zero.rank_base => Some(1000), "C++ normalizes a final zero base to its global default" }

        write_fixture! { def_dir.join("RankUS.txt") => b"base=250\nBase =300\nRecruit\n", "non-matching rank settings" };
        let defaulted = Definition::load_with_languages(
            &Group::open(&def_dir).expect("reopen definition"),
            &["US"],
        )
        .expect("reload definition");
        check_eq! { defaulted.rank_base => Some(1000), "the Base setting name and equals placement are exact" }
    }

    #[test]
    fn custom_rank_symbol_count_matches_invalid_and_saturated_cpp_cases() {
        let temp = tempdir().expect("tempdir");

        let invalid_dir = temp.path().join("InvalidRanks.c4d");
        fs::create_dir(&invalid_dir).expect("invalid definition directory");
        write_fixture! { invalid_dir.join("DefCore.txt") => b"[DefCore]\nid=INVR\n", "DefCore" };
        image::RgbaImage::from_pixel(4, 1, image::Rgba([255, 255, 255, 255]))
            .save(invalid_dir.join("Rank.png"))
            .expect("rank strip");
        write_fixture! { invalid_dir.join("RankUS.txt") => b"# no ordinary names\nBase=500\n*Unused %s\n", "invalid ranks" };
        let invalid = Definition::load_with_languages(
            &Group::open(&invalid_dir).expect("open invalid definition"),
            &["US"],
        )
        .expect("load invalid-rank definition");
        check_eq! { invalid.rank_symbol_count => Some(4), "C4RankSystem rejects a component without ordinary rank names" }
        check_eq! { invalid.rank_names => None }
        check_eq! { invalid.rank_base => None }

        let saturated_dir = temp.path().join("SaturatedRanks.c4d");
        fs::create_dir(&saturated_dir).expect("saturated definition directory");
        write_fixture! { saturated_dir.join("DefCore.txt") => b"[DefCore]\nid=SATR\n", "DefCore" };
        image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 255, 255, 255]))
            .save(saturated_dir.join("Rank.png"))
            .expect("rank strip");
        write_fixture! { saturated_dir.join("RankUS.txt") => b"Recruit\n*One %s\n*Two %s\n*Three %s\n", "saturated ranks" };
        let saturated = Definition::load_with_languages(
            &Group::open(&saturated_dir).expect("open saturated definition"),
            &["US"],
        )
        .expect("load saturated-rank definition");
        check_eq! { saturated.rank_symbol_count => Some(1), "C++ clamps the base rank symbol count to at least one" }
        check_eq! { saturated.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Recruit".to_string(), "One Recruit".to_string(), "Two Recruit".to_string(), "Three Recruit".to_string(),]) }

        write_fixture! { saturated_dir.join("RankUS.txt") => b"Recruit\n*Unterminated %s", "unterminated ranks" };
        let unterminated = Definition::load_with_languages(
            &Group::open(&saturated_dir).expect("reopen saturated definition"),
            &["US"],
        )
        .expect("load unterminated-rank definition");
        check_eq! { unterminated.rank_symbol_count => Some(2), "C++ ignores the final rank line when it has no CR or LF terminator" }
        check_eq! { unterminated.rank_names.as_ref().map(RankNameTable::resolved_names) => Some(vec!["Recruit".to_string()]) }
    }

    #[test]
    fn corrupt_rank_png_does_not_fall_through_to_rank_bmp() {
        definition_fixture_dir! { temp, def_dir => "BrokenRank.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=BRKN\n", "DefCore" };
        write_fixture! { def_dir.join("Rank.png") => b"not a png", "corrupt PNG" };
        image::RgbaImage::from_pixel(4, 1, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.bmp"))
            .expect("valid BMP fallback candidate");

        let definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("definition still loads");
        check! { definition.rank_symbols_image.is_none() }
        check_eq! { definition.rank_symbol_count => None }
    }

    #[test]
    fn rank_strip_narrower_than_one_square_phase_is_rejected() {
        definition_fixture_dir! { temp, def_dir => "NarrowRank.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=NARR\n", "DefCore" };
        image::RgbaImage::from_pixel(1, 2, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.png"))
            .expect("narrow rank strip");

        let definition = Definition::load(&Group::open(&def_dir).expect("open definition"))
            .expect("definition loads");
        check! { definition.rank_symbols_image.is_none() }
        check_eq! { definition.rank_symbol_count => None }
    }

    #[test]
    fn all_shipped_portrait_variants_are_retained_recursively() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content"));
        check! { root.is_dir(), "the initialized official content submodule must provide {}", root.display() }
        let mut definition_dirs = std::collections::BTreeSet::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let portrait =
                name.starts_with("portrait") && (name.ends_with(".png") || name.ends_with(".bmp"));
            if portrait {
                let parent = entry.path().parent().expect("portrait has parent");
                if parent.join("DefCore.txt").is_file() {
                    definition_dirs.insert(parent.to_path_buf());
                }
            }
        }

        let mut checked = 0;
        for directory in definition_dirs {
            let expected = std::fs::read_dir(&directory)
                .expect("read definition directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .filter_map(|entry| {
                    let path = entry.path();
                    let stem = path.file_stem()?.to_string_lossy();
                    let extension = path.extension()?.to_string_lossy();
                    (stem
                        .get(..8)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Portrait"))
                        && (extension.eq_ignore_ascii_case("png")
                            || extension.eq_ignore_ascii_case("bmp")))
                    .then(|| stem.get(8..).unwrap_or_default().to_string())
                })
                .collect::<Vec<_>>();
            let definition = Definition::load(&Group::open(&directory).expect("open definition"))
                .expect("load shipped definition");
            for name in &expected {
                check! { definition.portrait_graphics.iter().any(|portrait| portrait.name.eq_ignore_ascii_case(name)), "{} must retain Portrait{name}", directory.display() }
            }
            checked += expected.len();
        }
        // 85 before Queron 3 and the Metal & Magic packs it depends on were
        // vendored; those add 34 more. The per-directory assertion above is the
        // real check — this census only guards against the walk silently
        // covering less content than it should.
        check_eq! { checked => 119, "recursive shipped portrait census changed" }
    }

    #[test]
    fn defcore_id_uses_c4id_adapt_truncation_and_looks_like_id() {
        for (source, expected, valid) in [
            ("Clonk", "Clon", false),
            ("CLONKX", "CLON", true),
            ("1337", "1337", true),
            ("0000", "0000", false),
            ("3HUD", "3HUD", true),
            ("NONEfoo", "NONE", false),
            ("CL.ON", "CL", false),
            ("AB-C", "AB-C", false),
        ] {
            let core = parse_def_core(format!("[DefCore]\nid={source}\n").as_bytes())
                .expect("DefCore parses");
            check_eq! { core.id => expected, "source {source}" }
            check_eq! { core.has_valid_id() => valid, "source {source}" }
        }
    }

    #[test]
    fn parse_def_core_basic_fields() {
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk
            Category=C4D_Living|C4D_Object
            CrewMember=1
            BlitMode=2
            MoveToRange=17
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.id => "CLNK" }
        check_eq! { parsed.name.as_deref() => Some("Clonk") }
        check_eq! { parsed.category => (1 << 3) | (1 << 4) }
        check_eq! { parsed.crew_member => 1 }
        check_eq! { parsed.blit_mode => 2 }
        check_eq! { parsed.move_to_range => 17 }
        check_eq! { parsed.collection => None }
        check_eq! { parsed.collection_limit => 0 }
        check! { !parsed.collectible }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("default parses");
        check_eq! { defaulted.blit_mode => 0 }
        check_eq! { defaulted.move_to_range => 0 }

        let signed =
            parse_def_core(b"[DefCore]\nid=SIGN\nMoveToRange=-3\n").expect("signed range parses");
        check_eq! { signed.move_to_range => -3 }

        let raw_crew =
            parse_def_core(b"[DefCore]\nid=CREW\nCrewMember=-2\n").expect("raw crew value parses");
        check_eq! { raw_crew.crew_member => -2 }
    }

    #[test]
    fn def_core_ignores_non_oracle_alias_keys() {
        let aliases = parse_def_core(
            b"[DefCore]\nid=ALIA\nCanBeBase=1\nShape=-8,-16,16,32\nBurnTurnTo=FIRE\n",
        )
        .expect("unknown DefCore aliases are ignored");
        check! { !aliases.can_be_base }
        check_eq! { aliases.shape => None }
        check_eq! { aliases.burn_turn_to => None }
        check! { !aliases.reflected_ints.contains_key("Base") }

        let native = parse_def_core(
            b"[DefCore]\nid=NATV\nBase=-2\nWidth=16\nHeight=32\nOffset=-8,-16\nBurnTo=FIREtail\n",
        )
        .expect("native DefCore keys parse");
        check! { native.can_be_base }
        check_eq! { native.reflected_ints.get("Base") => Some(&-2) }
        check_eq! { native.shape => Some(PictureRect {x: -8, y: -16, width: 16, height: 32,}) }
        check_eq! { native.burn_turn_to.as_deref() => Some("FIRE") }
    }

    #[test]
    fn parse_def_core_rct_all_skips_leading_and_preserves_trailing_whitespace() {
        let parsed = parse_def_core(
            b"[DefCore]\nid=RCTA\nName= \tBar \t\nTimerCall= \tFoo \t\n\
              ColorByMaterial= \tGranite \t\nBurnTo=BURN \t\nConstructTo=DONE \t\n",
        )
        .expect("DefCore RCT_All strings parse");

        check_eq! { parsed.name.as_deref() => Some("Bar \t") }
        check_eq! { parsed.timer_call.as_deref() => Some("Foo \t") }
        check_eq! { parsed.color_by_material => "Granite \t" }
        check_eq! { parsed.burn_turn_to.as_deref() => Some("BURN") }
        check_eq! { parsed.build_turn_to.as_deref() => Some("DONE") }
    }

    #[test]
    fn def_core_empty_name_overrides_undefined_default() {
        for (source, expected) in [
            ("Name=", Some("")),
            ("Name= \t", Some("")),
            ("Name=\nName=Later", Some("")),
            ("name=wrong case", None),
            ("", None),
        ] {
            let parsed = parse_def_core(format!("[DefCore]\nid=EMTY\n{source}\n").as_bytes())
                .expect("DefCore name fixture parses");
            check_eq! { parsed.name.as_deref() => expected, "source {source:?}" }
        }
    }

    #[test]
    fn parse_def_core_preserves_native_bytes_from_shipped_hut() {
        let parsed = parse_def_core(include_bytes!(
            "../../../content/Objects.c4d/Structures.c4d/Hut2.c4d/DefCore.txt"
        ))
        .expect("shipped Hut2 DefCore parses");
        let name = parsed.name.as_deref().expect("Hut2 carries a core name");

        check_eq! { clonk_script::c4_string_bytes(name) => b"Holzh\xfctte" }
        check! { !name.contains('\u{fffd}') }
        check_ne! { name => "Holzhütte", "raw 0xfc is not UTF-8 ü" }
    }

    #[test]
    fn parse_def_core_preserves_native_bytes_in_rct_all_fields() {
        let parsed = parse_def_core(
            b"[DefCore]\nid=BYTE\nColorByMaterial=Rock\x80\nTimerCall=F\xfcnc\x80\n",
        )
        .expect("native-byte DefCore strings parse");

        check_eq! { clonk_script::c4_string_bytes(&parsed.color_by_material) => b"Rock\x80" }
        check_eq! { clonk_script::c4_string_bytes(parsed.timer_call.as_deref().expect("TimerCall")) => b"F\xfcnc\x80" }
    }

    #[test]
    fn parse_def_core_truncates_native_strings_at_byte_and_nul_boundaries() {
        let parsed = parse_def_core(b"[DefCore]\nid=BYTE\nName=pre\xfc\0ignored\nTimerCall=Late\n")
            .expect("NUL-terminated DefCore parses");
        check_eq! { clonk_script::c4_string_bytes(parsed.name.as_deref().expect("Name")) => b"pre\xfc" }
        check_eq! { parsed.timer_call => None }

        let mut bounded = b"[DefCore]\nid=BYTE\nColorByMaterial=".to_vec();
        bounded.extend_from_slice(b"12345678901234\xc3\xbc\nTimerCall=");
        bounded.extend_from_slice(b"1234567890123456789012345678\xc3\xbc\n");
        let parsed = parse_def_core(&bounded).expect("bounded native strings parse");
        check_eq! { clonk_script::c4_string_bytes(&parsed.color_by_material) => b"12345678901234\xc3" }
        check_eq! { clonk_script::c4_string_bytes(parsed.timer_call.as_deref().expect("TimerCall")) => b"1234567890123456789012345678\xc3" }
    }

    #[test]
    fn parse_def_core_keeps_raw_and_utf8_names_byte_distinct() {
        let raw =
            parse_def_core(b"[DefCore]\nid=BYTE\nName=\x80\n").expect("raw-byte DefCore parses");
        let utf8 = parse_def_core(b"[DefCore]\nid=BYTE\nName=\xe2\x82\xac\n")
            .expect("UTF-8 DefCore parses");
        let raw = raw.name.as_deref().expect("raw Name");
        let utf8 = utf8.name.as_deref().expect("UTF-8 Name");

        check_eq! { clonk_script::c4_string_bytes(raw) => b"\x80" }
        check_eq! { clonk_script::c4_string_bytes(utf8) => b"\xe2\x82\xac" }
        check_ne! { raw => utf8 }
    }

    #[test]
    fn parse_def_core_stdcompiler_numeric_prefixes_and_radices() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=LEXR
Mass=100abc
Vertices=3 ; comment
BlitMode=0X11
Value=$FF
Growth=0b101
MoveToRange=4294967297

[Physical]
Jump=40000junk
"#,
        )
        .expect("DefCore numeric prefixes parse");

        check_eq! { parsed.mass => 100 }
        check_eq! { parsed.vertices.len() => 3 }
        check_eq! { parsed.blit_mode => 17 }
        check_eq! { parsed.value => 0, "$ is not a C++ integer prefix" }
        check_eq! { parsed.growth => 0, "0b consumes only the leading zero" }
        let narrowed_overflow = if std::mem::size_of::<std::os::raw::c_long>() == 8 {
            1
        } else {
            i32::MAX
        };
        check_eq! { parsed.move_to_range => narrowed_overflow }
        check_eq! { parsed.physical.jump => 40_000 }

        for (raw, expected) in [
            ("0X65junk", 101),
            ("$FF", 100),
            ("0b101", 0),
            ("-1", u32::MAX),
        ] {
            let scale = parse_def_core(format!("[DefCore]\nid=SCAL\nScale={raw}\n").as_bytes())
                .expect("Scale DefCore parses");
            check_eq! { scale.graphics_scale => expected, "Scale={raw}" }
        }
    }

    #[test]
    fn parse_def_core_uses_cpp_boolean_only_for_boolean_typed_fields() {
        for value in ["true", "yes"] {
            let parsed = parse_def_core(format!("[DefCore]\nid=REBY\nRebuy={value}\n").as_bytes())
                .expect("Rebuy DefCore parses");
            check! { !parsed.rebuyable, "int32 Rebuy={value} defaults to zero" }
        }

        let gold =
            parse_def_core(b"[DefCore]\nid=GOLD\nBaseAutoSell=2\n").expect("GOLD DefCore parses");
        check! { gold.base_auto_sell, "invalid Boolean text restores the GOLD-specific default" }
        let ordinary = parse_def_core(b"[DefCore]\nid=ROCK\nBaseAutoSell=2\n")
            .expect("ordinary DefCore parses");
        check! { !ordinary.base_auto_sell }
    }

    #[test]
    fn parse_def_core_stdcompiler_boolean_grammar() {
        for (raw, expected) in [
            ("1x", Some(true)),
            ("0x", Some(false)),
            ("truejunk", Some(true)),
            ("falsehood", Some(false)),
            ("10", None),
            ("00", None),
            ("TRUE", None),
            ("yes", None),
            ("on", None),
            (" true", None),
        ] {
            check_eq! { parse_bool(raw) => expected, "Boolean `{raw}`" }
        }
    }

    #[test]
    fn def_core_load_adjusts_crew_category_before_sort_default_like_cpp() {
        let temp = tempdir().expect("tempdir");
        let load = |directory: &str, source: &str| {
            let path = temp.path().join(directory);
            fs::create_dir(&path).expect("definition directory");
            write_fixture! { path.join("DefCore.txt") => source, "write DefCore" };
            let group = Group::open(&path).expect("open definition group");
            DefCore::load(&group).expect("load DefCore")
        };

        let ordinary = load(
            "Ordinary.c4d",
            "[DefCore]\nid=ORDN\nCategory=C4D_Living\nCrewMember=0\n",
        );
        check_eq! { ordinary.category => 1 << 3, "CrewMember=0 changes nothing" }

        let derived = load("Derived.c4d", "[DefCore]\nid=CREW\nCrewMember=-2\n");
        check_eq! { derived.category => C4D_CREW_MEMBER | 1, "the crew bit is present before the missing sort bit defaults to StaticBack" }

        let explicit = load(
            "Explicit.c4d",
            "[DefCore]\nid=EXPL\nCategory=C4D_CrewMember|C4D_Object\nCrewMember=0\n",
        );
        check_eq! { explicit.category => C4D_CREW_MEMBER | (1 << 4), "CrewMember=0 never clears an explicit category bit" }
    }

    #[test]
    fn parse_def_core_retains_the_five_component_cpp_version() {
        // C4DefCore::CompileFunc stores Version in the five-slot rC4XVer
        // array, zero-filling omitted components (src/C4Def.cpp:124,254).
        let parsed = parse_def_core(b"[DefCore]\nid=VERS\nVersion=4,9,1,3,27\n")
            .expect("versioned DefCore parses");
        check_eq! { parsed.version => [4, 9, 1, 3, 27] }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("defaults parse");
        check_eq! { defaulted.version => [0; 5] }
    }

    #[test]
    fn def_core_integer_arrays_stop_on_cpp_separator_failure() {
        let parsed = parse_def_core(
            b"[DefCore]\n\
              id=ARRY\n\
              Version=4,9 5\n\
              Width=10\n\
              Height=20\n\
              Offset=1;2\n\
              Vertices=3\n\
              VertexX=0 5 -5\n\
              VertexY=1,x,3\n\
              Entrance=7 8 9 10\n\
              Picture=0 0 64 64\n",
        )
        .expect("array probe DefCore parses");

        check_eq! { parsed.version => [4, 9, 0, 0, 0] }
        check_eq! { parsed.vertex_slots.iter().take(3).map(|vertex| vertex.x).collect::<Vec<_>>() => [0, 0, 0] }
        check_eq! { parsed.vertex_slots.iter().take(3).map(|vertex| vertex.y).collect::<Vec<_>>() => [1, 0, 0] }
        check_eq! { parsed.shape => Some(PictureRect {x: 1, y: 0, width: 10, height: 20,}) }
        check_eq! { parsed.entrance => Some(PictureRect {x: 7, y: 0, width: 0, height: 0,}) }
        check_eq! { parsed.picture => Some(PictureRect {x: 0, y: 0, width: 0, height: 0,}) }
    }

    #[test]
    fn parse_def_core_pathfinder_and_transfer_zone_policy() {
        // C4DefCore::CompileFunc reads both fields as integer defaults of
        // zero (C4Def.cpp:399,415); command code treats either sign of a
        // nonzero Pathfinder as enabled and SetLevel clamps it later.
        let parsed = parse_def_core(b"[DefCore]\nid=ROUT\nPathfinder=-4\nNoTransferZones=-2\n")
            .expect("pathfinder DefCore parses");
        check_eq! { parsed.pathfinder => -4 }
        check_eq! { parsed.no_transfer_zones => -2 }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("defaults parse");
        check_eq! { defaulted.pathfinder => 0 }
        check_eq! { defaulted.no_transfer_zones => 0 }
    }

    #[test]
    fn parse_def_core_no_push_enter_preserves_signed_value_and_default() {
        // C4DefCore::CompileFunc stores NoPushEnter as int32_t with zero as
        // its default; command code treats either sign as enabled.
        let parsed = parse_def_core(b"[DefCore]\nid=LOCK\nNoPushEnter=-2\n")
            .expect("NoPushEnter DefCore parses");
        check_eq! { parsed.no_push_enter => -2 }

        let defaulted = parse_def_core(b"[DefCore]\nid=OPEN\n").expect("default DefCore parses");
        check_eq! { defaulted.no_push_enter => 0 }
    }

    #[test]
    fn parse_def_core_no_sell_preserves_signed_value_and_default() {
        // C4DefCore::CompileFunc stores NoSell as int32_t with zero as its
        // default; SellFromBase treats either sign of a nonzero value as set.
        let parsed =
            parse_def_core(b"[DefCore]\nid=LOCK\nNoSell=-2\n").expect("NoSell DefCore parses");
        check_eq! { parsed.no_sell => -2 }

        let defaulted = parse_def_core(b"[DefCore]\nid=OPEN\n").expect("default DefCore parses");
        check_eq! { defaulted.no_sell => 0 }
    }

    #[test]
    fn parse_def_core_allow_picture_stack_bitfield() {
        // C4Def::CompileFunc parses AllowPictureStack through the APS_* table
        // (src/C4Def.cpp:419-429; src/C4Constants.h:301-309).
        let parsed = parse_def_core(
            b"[DefCore]\nid=STACK\nAllowPictureStack=APS_Color|APS_Graphics|APS_Name|APS_Overlay\n",
        )
        .expect("DefCore parses");
        check_eq! { parsed.allow_picture_stack => APS_COLOR | APS_GRAPHICS | APS_NAME | APS_OVERLAY }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("DefCore parses");
        check_eq! { defaulted.allow_picture_stack => 0 }
        check_eq! { defaulted.graphics_scale => 100 }

        let scaled =
            parse_def_core(b"[DefCore]\nid=SCALE\nScale=125\n").expect("graphics scale parses");
        check_eq! { scaled.graphics_scale => 125 }
    }

    #[test]
    fn hd_crew_pack_defcore_and_action_facets_parse() {
        // Verbatim from a Scale=300 crew pack emitted by RenderClonkAddon, the
        // shape every high-resolution definition takes. No shipped definition
        // sets DefCore Scale, so this path had no end-to-end coverage: the
        // parse of `Scale=` was pinned in isolation and the six-component
        // `Facet=` was pinned only through synthetic rects.
        //
        // Two things have to hold together for such a pack to render. The
        // sheet grows but every ActMap number stays LOGICAL, so `Facet=` must
        // keep its unscaled values; and the fifth and sixth components are
        // C4TargetRect's tx/ty (C4Def.h:158), which HD packs use for the
        // headroom their taller cells need and which a four-component parse
        // would silently drop.
        let core = parse_def_core(
            b"[DefCore]\nid=CLNK\nName=Clonk\nWidth=16\nHeight=20\n\
              Offset=-8, -10\nPicture=280,242,32,40\nScale=300\n",
        )
        .expect("HD crew DefCore parses");

        check_eq! { core.graphics_scale => 300 }
        let shape = core
            .shape
            .expect("Width/Height give the definition a shape");
        assert_eq!(
            (shape.width, shape.height),
            (16, 20),
            "the collision box stays in game units at any Scale — scaling it \
             would move the object, not just its picture"
        );

        let walk = parse_action_facet("0,0,16,22,0,-2").expect("HD walk facet parses");
        check_eq! { (walk.x, walk.y, walk.width, walk.height) => (0, 0, 16, 22), "the facet rect stays logical; only the sheet is scaled" }
        check_eq! { (walk.target_x, walk.target_y) => (0, -2), "the fifth and sixth components are C4TargetRect tx/ty, not padding" }

        // The very next action on the same sheet uses a different cell size —
        // which is why cell geometry can only come from the ActMap.
        let scale_action = parse_action_facet("0,22,20,22,-2,-1").expect("HD scale facet parses");
        check_eq! { (scale_action.width, scale_action.height) => (20, 22), "actions on one sheet legitimately differ in cell size" }
    }

    #[test]
    fn line_compiles_named_tokens_as_a_bitfield() {
        // C4Def::CompileFunc passes Line through mkBitfieldAdapt with the
        // C4D_Line_* table (C4Def.cpp:319-333). DrainPipe.c4d encodes the
        // drain value 3 as Power(1)|Source(2), not the Drain alias.
        let parsed = parse_def_core(b"[DefCore]\nid=DPIP\nLine=C4D_LinePower|C4D_LineSource\n")
            .expect("drain-pipe DefCore parses");

        check_eq! { parsed.line => 3 }
    }

    #[test]
    fn def_core_bitfields_match_cpp_unknown_case_and_separator_rules() {
        let temp = tempdir().expect("tempdir");
        let load = |directory: &str, source: &str| {
            let path = temp.path().join(directory);
            fs::create_dir(&path).expect("definition directory");
            write_fixture! { path.join("DefCore.txt") => source, "write DefCore" };
            let group = Group::open(&path).expect("open definition group");
            Definition::load(&group).expect("unknown bit names only warn")
        };

        let category = load(
            "Category.c4d",
            "[DefCore]\nid=STRU\nCategory=C4D_Structure|C4D_Bogus\n",
        );
        check_eq! { category.core.category => 1 << 1 }

        let line_connect = load(
            "LineConnect.c4d",
            "[DefCore]\nid=LINE\nCategory=C4D_Object\nLineConnect=C4D_PowerInput|Nonsense\n",
        );
        check_eq! { line_connect.core.line_connect => 1 }

        let stopped = load(
            "Stopped.c4d",
            "[DefCore]\nid=SPAC\nCategory=C4D_Living C4D_Object\n",
        );
        check_eq! { stopped.core.category => 1 << 3 }

        let wrong_case = parse_def_core(b"[DefCore]\nid=CASE\nCategory=c4d_structure\n")
            .expect("wrong-case name is a valid unknown identifier");
        check_eq! { wrong_case.category => 0 }
        let wrong_case_loaded = load(
            "WrongCase.c4d",
            "[DefCore]\nid=CASE\nCategory=c4d_structure\n",
        );
        check_eq! { wrong_case_loaded.core.category => 1, "the later C4DefCore::Load sort default still adds StaticBack" }

        let shared = parse_def_core(
            br#"[DefCore]
id=BITS
Category=C4D_Structure|Bogus|C4D_Goal
Line=C4D_LinePower|Bogus|C4D_LineSource
LineConnect=C4D_PowerInput|Bogus|C4D_LiquidOutput
GrabPutGet=C4D_GrabPut|Bogus|C4D_GrabGet
AllowPictureStack=APS_Color|Bogus|APS_Overlay
HideHUDBars=Energy|Bogus|Breath
HideHUDElements=Portrait|Bogus|Inventory
"#,
        )
        .expect("unknown identifiers do not abort any DefCore bitfield");
        check_eq! { shared.category => (1 << 1) | (1 << 5) }
        check_eq! { shared.line => 3 }
        check_eq! { shared.line_connect => 1 | (1 << 3) }
        check_eq! { shared.grab_put_get => 3 }
        check_eq! { shared.allow_picture_stack => APS_COLOR | APS_OVERLAY }
        check_eq! { shared.hide_hud_bars => 1 | 4 }
        check_eq! { shared.hide_hud_elements => 1 | 32 }

        let non_pipe = parse_def_core(
            b"[DefCore]\nid=STOP\nCategory=C4D_Structure+C4D_Goal\nLineConnect=C4D_PowerInput,C4D_PowerOutput\n",
        )
        .expect("separator mismatches end the bitfield");
        check_eq! { non_pipe.category => 1 << 1 }
        check_eq! { non_pipe.line_connect => 1 }

        let malformed = parse_def_core(b"[DefCore]\nid=ZERO\nCategory=C4D_Structure||C4D_Goal\n")
            .expect("the outer default adaptor handles malformed bitfields");
        check_eq! { malformed.category => 0 }
    }

    #[test]
    fn def_core_reports_unknown_bit_diagnostics_in_source_order() {
        let mut diagnostics = Vec::new();
        parse_def_core_with_diagnostics(
            b"[DefCore]\nid=BITS\nCategory=First|C4D_Object\nLineConnect=Second\n",
            &mut |diagnostic| diagnostics.push(diagnostic),
        )
        .expect("unknown bit names remain non-fatal");

        assert_eq!(
            diagnostics,
            [
                ResourceLoadDiagnostic::UnknownDefinitionBitName {
                    bit_name: "First".to_string(),
                },
                ResourceLoadDiagnostic::UnknownDefinitionBitName {
                    bit_name: "Second".to_string(),
                },
            ]
        );
    }

    #[test]
    fn definition_loads_nonempty_legacy_us_description() {
        // C4Def loads Desc{}.txt into C4Def::Desc and trims it before
        // exposing C4Def::GetDesc (C4Def.cpp:713-717). The Context menu
        // adds Info only for a nonempty GetDesc (C4ObjectMenu.cpp:410-423).
        definition_fixture_dir! { temp, def_dir => "Hut3.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=HUT3\n", "DefCore" };
        write_fixture! { def_dir.join("DescUS.txt") => b"  A safe home base.\r\n", "US description" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");

        check_eq! { definition.description() => Some("A safe home base.") }

        // StdStrBuf::TrimSpaces applies bytewise C-locale isspace; a CP1252
        // non-breaking space survives even after EnsureUnicode converts it.
        write_fixture! { def_dir.join("DescUS.txt") => b"\xa0Kept\xa0", "non-ASCII description whitespace" };
        let definition = Definition::load(&group).expect("reload definition");
        check_eq! { definition.description() => Some("\u{a0}Kept\u{a0}") }
    }

    #[test]
    fn definition_description_follows_language_order_without_plain_fallback() {
        let temp = tempdir().expect("tempdir");
        let both_dir = temp.path().join("Both.c4d");
        fs::create_dir(&both_dir).expect("both-language definition directory");
        write_fixture! { both_dir.join("DefCore.txt") => b"[DefCore]\nid=BOTH\n", "DefCore" };
        write_fixture! { both_dir.join("DescDE.txt") => b"  Deutsche Beschreibung  ", "German description" };
        write_fixture! { both_dir.join("DescUS.txt") => b"English description", "US description" };
        let both = Group::open(&both_dir).expect("open both-language definition");
        let german = Definition::load_with_languages(&both, &["DE", "US"])
            .expect("load German-first definition");
        check_eq! { german.description() => Some("Deutsche Beschreibung") }

        let de_only_dir = temp.path().join("GermanOnly.c4d");
        fs::create_dir(&de_only_dir).expect("German-only definition directory");
        write_fixture! { de_only_dir.join("DefCore.txt") => b"[DefCore]\nid=DEON\n", "DefCore" };
        write_fixture! { de_only_dir.join("DescDE.txt") => b"Nur Deutsch", "German description" };
        write_fixture! { de_only_dir.join("Desc.txt") => b"Plain fallback must not load", "plain description" };
        let de_only = Group::open(&de_only_dir).expect("open German-only definition");
        let us_only =
            Definition::load_with_languages(&de_only, &["US"]).expect("load US-only sequence");
        check_eq! { us_only.description() => None }
        let german_fallback = Definition::load_with_languages(&de_only, &["US", "DE"])
            .expect("load German second candidate");
        check_eq! { german_fallback.description() => Some("Nur Deutsch") }
    }

    #[test]
    fn definition_clonk_names_follow_language_sequence_before_plain_fallback() {
        // C4Def::Load gates on a local ClonkNames*.txt marker, then LoadEx
        // tries ClonkNames{lang}.txt for each two-byte language code before
        // the plain component (C4Def.cpp:641-657; C4ComponentHost.cpp:65-94).
        definition_fixture_dir! { temp, def_dir => "Crew.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=CREW\n", "DefCore" };
        write_fixture! { def_dir.join("ClonkNamesDE.txt") => b"J\xfcrgen\n", "German clonk names" };
        write_fixture! { def_dir.join("ClonkNamesUS.txt") => b"John\n", "US clonk names" };
        write_fixture! { def_dir.join("ClonkNamesD.txt") => b"Nul Code\n", "single-byte language code clonk names" };
        write_fixture! { def_dir.join("ClonkNames.txt") => b"Plain\n", "plain clonk names" };
        let group = Group::open(&def_dir).expect("open definition");

        let german = Definition::load_with_languages(&group, &["DE", "US"])
            .expect("load German-first definition");
        check_eq! { german.clonk_names.as_deref() => Some("Jürgen\n") }

        let truncated = Definition::load_with_languages(&group, &["DE-extra"])
            .expect("language code truncates to two native bytes");
        check_eq! { truncated.clonk_names.as_deref() => Some("Jürgen\n") }

        let nul_code = Definition::load_with_languages(&group, &["D\0E"])
            .expect("language code stops at its native NUL");
        check_eq! { nul_code.clonk_names.as_deref() => Some("Nul Code\n") }

        let plain = Definition::load_with_languages(&group, &["FI"]).expect("load plain fallback");
        check_eq! { plain.clonk_names.as_deref() => Some("Plain\n") }

        write_fixture! { def_dir.join("ClonkNamesUS.txt") => b"Before\0After\n", "NUL-terminated clonk names" };
        let nul_terminated = Definition::load_with_languages(&group, &["US"])
            .expect("load NUL-terminated component");
        check_eq! { nul_terminated.clonk_names.as_deref() => Some("Before") }

        write_fixture! { def_dir.join("ClonkNamesUS.txt") => b"\0After", "leading-NUL clonk names" };
        let empty_owned = Definition::load_with_languages(&group, &["US"])
            .expect("a nonzero component with an empty C string still loads");
        check_eq! { empty_owned.clonk_names.as_deref() => Some("") }
    }

    #[test]
    fn definition_description_preserves_componenthost_empty_candidate_rules() {
        let temp = tempdir().expect("tempdir");

        let advancing_dir = temp.path().join("Advancing.c4d");
        fs::create_dir(&advancing_dir).expect("advancing definition directory");
        write_fixture! { advancing_dir.join("DefCore.txt") => b"[DefCore]\nid=ADVN\n", "DefCore" };
        write_fixture! { advancing_dir.join("DescUS.txt") => [], "empty US description" };
        write_fixture! { advancing_dir.join("descde.TXT") => b"Gemischte Schreibweise", "mixed-case German description" };
        let advancing = Group::open(&advancing_dir).expect("open advancing definition");
        let definition = Definition::load_with_languages(&advancing, &["US", "DE"])
            .expect("zero-byte candidate advances");
        check_eq! { definition.description() => Some("Gemischte Schreibweise") }

        let blocking_dir = temp.path().join("Blocking.c4d");
        fs::create_dir(&blocking_dir).expect("blocking definition directory");
        write_fixture! { blocking_dir.join("DefCore.txt") => b"[DefCore]\nid=BLOK\n", "DefCore" };
        write_fixture! { blocking_dir.join("DescUS.txt") => b" \r\n\t", "whitespace US description" };
        write_fixture! { blocking_dir.join("DescDE.txt") => b"Must not load", "German description" };
        let blocking = Group::open(&blocking_dir).expect("open blocking definition");
        let definition = Definition::load_with_languages(&blocking, &["US", "DE"])
            .expect("whitespace candidate loads then trims");
        check_eq! { definition.description() => None }

        let plain_dir = temp.path().join("Plain.c4d");
        fs::create_dir(&plain_dir).expect("plain definition directory");
        write_fixture! { plain_dir.join("DefCore.txt") => b"[DefCore]\nid=PLAN\n", "DefCore" };
        write_fixture! { plain_dir.join("Desc.txt") => b"Explicit empty-code description", "plain description" };
        let plain = Group::open(&plain_dir).expect("open plain definition");
        let definition = Definition::load_with_languages(&plain, &[] as &[&str])
            .expect("empty language sequence tries one empty code");
        check_eq! { definition.description() => Some("Explicit empty-code description") }
    }

    #[test]
    fn definition_loads_first_language_description_from_language_pack() {
        // Candidate language order precedes local-or-pack group priority in
        // C4ComponentHost::LoadEx.
        let temp = tempdir().expect("tempdir");
        let content = temp.path().join("content");
        let def_dir = content.join("Hut3.c4d");
        fs::create_dir_all(&def_dir).expect("definition directory");
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=HUT3\n", "DefCore" };
        write_fixture! { def_dir.join("DescUS.txt") => b"Local English description", "local US description" };

        let language_container = temp.path().join("Language.c4g");
        let pack_def = language_container.join("Pack.c4g/Hut3.c4d");
        fs::create_dir_all(&pack_def).expect("pack definition directory");
        write_fixture! { pack_def.join("DescDE.txt") => b"Falsche Sprache", "German pack description" };
        write_fixture! { pack_def.join("DescUS.txt") => b"  Packed home base.\r\n", "US pack description" };

        let packs = crate::LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            std::slice::from_ref(&content),
        );
        let group = Group::open(&def_dir).expect("open definition");
        let components = packs.component_groups(&group, None, None);
        let definition =
            Definition::load_with_languages_and_components(&group, &["DE", "US"], &components)
                .expect("load pack-described definition");

        check_eq! { definition.description() => Some("Falsche Sprache") }
    }

    #[test]
    fn definition_name_uses_localized_names_component() {
        // C4Def::Load loads Names{}.txt|Names.txt after DefCore and replaces
        // C4Def::Name with the first language-sequence match
        // (C4Def.cpp:635-639; C4ComponentHost.cpp:238-260). HUT3 therefore
        // presents as "Cabin", not its DefCore fallback "Hut".
        definition_fixture_dir! { temp, def_dir => "Hut3.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=HUT3\nName=Hut\n", "DefCore" };
        write_fixture! { def_dir.join("Names.txt") => b"DE:H\xfctte\r\nUS:Cabin\r\n", "localized names" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");

        check_eq! { definition.core.name.as_deref() => Some("Cabin") }
        let german = Definition::load_with_languages(&group, &["DE", "US"])
            .expect("load German definition name");
        check_eq! { german.core.name.as_deref() => Some("Hütte") }
        let truncated = Definition::load_with_languages(&group, &["DE-extra", "US"])
            .expect("truncate the definition-name language code too");
        check_eq! { truncated.core.name.as_deref() => Some("Hütte") }
        let empty = Definition::load_with_languages(&group, &[] as &[&str])
            .expect("empty language sequence uses one empty code");
        check_eq! { empty.core.name.as_deref() => Some("Hütte") }
    }

    #[test]
    fn definition_name_line_end_prefers_any_later_cr() {
        definition_fixture_dir! { temp, def_dir => "MixedLines.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=MIXD\nName=Fallback\n", "DefCore" };
        write_fixture! { def_dir.join("Names.txt") => b"US:Cabin\nDE:Huette\r\n", "mixed-line-ending names" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition =
            Definition::load_with_languages(&group, &["US", "DE"]).expect("load definition name");

        check_eq! { definition.core.name.as_deref() => Some("Cabin\nDE:Huette") }
    }

    #[test]
    fn definition_name_empty_first_language_value_wins() {
        definition_fixture_dir! { temp, def_dir => "EmptyName.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=EMNM\nName=Core Name\n", "DefCore" };
        write_fixture! { def_dir.join("Names.txt") => b"US:\r\nDE:Huette\r\n", "localized names" };

        let group = Group::open(&def_dir).expect("open definition");
        let empty = Definition::load_with_languages(&group, &["US", "DE"])
            .expect("empty first language name");
        check_eq! { empty.core.name.as_deref() => Some("") }

        let missing =
            Definition::load_with_languages(&group, &["FR"]).expect("missing language name");
        check_eq! { missing.core.name.as_deref() => Some("Core Name") }
    }

    #[test]
    fn zero_size_definition_text_components_follow_load_entry_string() {
        let temp = tempdir().expect("tempdir");

        let empty_core_dir = temp.path().join("EmptyCore.c4d");
        fs::create_dir(&empty_core_dir).expect("empty-core definition directory");
        write_fixture! { empty_core_dir.join("DefCore.txt") => [], "empty DefCore" };
        let empty_core = Group::open(&empty_core_dir).expect("open empty-core definition");
        check! { matches!(DefCore::load(&empty_core), Err(DefinitionError::DefCoreMissing)) }

        let def_dir = temp.path().join("EmptyComponents.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=EMTY\nName=Core Name\n", "DefCore" };
        write_fixture! { def_dir.join("ActMap.txt") => [], "empty ActMap" };
        write_fixture! { def_dir.join("NamesUS.txt") => [], "empty localized names" };
        write_fixture! { def_dir.join("Names.txt") => b"US:Fallback Name\n", "fallback names" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition =
            Definition::load_with_languages(&group, &["US"]).expect("empty components are absent");
        check! { definition.action_map.is_none() }
        check_eq! { definition.core.name.as_deref() => Some("Fallback Name") }

        write_fixture! { def_dir.join("ActMap.txt") => b"malformed action map\n", "malformed nonempty ActMap" };
        let group = Group::open(&def_dir).expect("reopen definition");
        check! { matches!(Definition::load_with_languages(&group, &["US"]), Err(DefinitionError::ActMapParse(_))) }
    }

    #[test]
    fn nonempty_names_component_still_blocks_filename_fallback() {
        definition_fixture_dir! { temp, def_dir => "NamesBlock.c4d" };
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=NBLK\nName=Core Name\n", "DefCore" };
        write_fixture! { def_dir.join("NamesUS.txt") => b"DE:Deutsch\n", "nonmatching localized names" };
        write_fixture! { def_dir.join("Names.txt") => b"US:Fallback Name\n", "fallback names" };

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load_with_languages(&group, &["US"])
            .expect("nonempty selected component loads");

        check_eq! { definition.core.name.as_deref() => Some("Core Name") }
    }

    #[test]
    fn load_definition_with_scripts_and_actions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Example.ocd");
        fs::create_dir(&def_dir).unwrap();
        write_fixture! { def_dir.join("DefCore.txt") => br#"[DefCore]
id=EXMP
Name=Example
Category=C4D_Object
CrewMember=0
"# };
        write_fixture! { def_dir.join("Script.c") => b"func Initialize() {}\n" };
        write_fixture! { def_dir.join("ActMap.txt") => br#"
[Action]
Name=Idle
Procedure=Walk
Length=20
NextAction=Idle
StartCall=OnIdleStart
EndCall=OnIdleEnd
"# };

        let group = Group::open(&def_dir).unwrap();
        let def = Definition::load(&group).expect("definition load succeeds");
        check_eq! { def.core.id => "EXMP" }
        check_eq! { def.core.name.as_deref() => Some("Example") }
        check_eq! { def.core.category => 1 << 4 }
        check_eq! { def.core.crew_member => 0 }
        check_eq! { def.script.files.len() => 1 }
        check! { def.script.combined.contains("Initialize") }
        let action_map = def.action_map.expect("action map present");
        check! { action_map.default_action.is_none() }
        let idle = action_map.get("Idle").expect("idle action present");
        check_eq! { idle.procedure.as_deref() => Some("Walk") }
        check_eq! { idle.length => Some(20) }
        check_eq! { idle.next_action.as_deref() => Some("Idle") }
        check_eq! { idle.start_call.as_deref() => Some("OnIdleStart") }
        check_eq! { idle.end_call.as_deref() => Some("OnIdleEnd") }
    }

    #[test]
    fn cross_map_act_map_maps_procedures_and_next_actions_like_cpp() {
        // C4Def::CrossMapActMap (C4Def.cpp:773-799): Procedure maps
        // case-SENSITIVELY against the uppercase ProcedureName table
        // (C4Def.cpp:38-58) with DFA_NONE fallback; NextAction maps "Hold"
        // case-insensitively to ActHold, otherwise case-SENSITIVELY against
        // the action names — the overwrite loop means the LAST duplicate
        // wins; no match leaves the ActIdle default.
        let data = br#"
[Action]
Name=Walk
Procedure=WALK
NextAction=hOLD

[Action]
Name=Fall
Procedure=walk
NextAction=Walk

[Action]
Name=Spin
NextAction=spin

[Action]
Name=Dup

[Action]
Name=Dup

[Action]
Name=Ref
Procedure=FLIGHT
NextAction=Dup
"#;
        let map = parse_act_map(data).expect("act map parsed");
        // file order preserved, duplicates kept (C++ array semantics)
        let order: Vec<&str> = map.actions.iter().map(|(name, _)| name.as_str()).collect();
        check_eq! { order => ["Walk", "Fall", "Spin", "Dup", "Dup", "Ref"] }

        let walk = map.get("Walk").expect("walk present");
        check_eq! { walk.procedure_index => 0, "WALK → DFA_WALK" }
        check_eq! { walk.next_action_index => ACT_HOLD, "Hold is case-insensitive" }

        let fall = map.get("Fall").expect("fall present");
        check_eq! { fall.procedure_index => DFA_NONE, "lowercase 'walk' does not match the case-sensitive table" }
        check_eq! { fall.next_action_index => 0, "NextAction=Walk → index 0" }

        let spin = map.get("Spin").expect("spin present");
        check_eq! { spin.next_action_index => ACT_IDLE, "case-sensitive miss leaves ActIdle" }

        let reference = map.get("Ref").expect("ref present");
        check_eq! { reference.procedure_index => 1, "FLIGHT → DFA_FLIGHT" }
        check_eq! { reference.next_action_index => 4, "last duplicate wins (C4Def.cpp:789-791 overwrite loop)" }
    }

    #[test]
    fn parse_real_bird_defcore_physical_float() {
        // The real CRLF content file pins the [Physical] Float=200 parse that
        // drives the DFA_FLOAT speed clamp.
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/Objects.c4d/Animals.c4d/Bird.c4d/DefCore.txt"
        ))
        .expect("initialized official content submodule provides Bird DefCore.txt");
        let core = parse_def_core(&bytes).expect("parses");
        check_eq! { core.physical.float => 200, "[Physical] Float=200" }
        check_eq! { core.physical.energy => 40000, "[Physical] Energy=40000" }
    }

    #[test]
    fn parse_def_core_physical_section() {
        // C4PhysicalInfo via the [Physical] DefCore section
        // (C4Def.cpp:459-460, name map C4InfoCore.cpp:181-205); defaults are
        // all zero (C4InfoCore.cpp:239-242).
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk

            [Physical]
            Energy=50000
            Walk=35000
            Fight=20000
            CanScale=1
            CorrosionResist=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.physical.energy => 50_000 }
        check_eq! { parsed.physical.walk => 35_000 }
        check_eq! { parsed.physical.fight => 20_000 }
        check_eq! { parsed.physical.can_scale => 1 }
        check_eq! { parsed.physical.corrosion_resist => 1 }
        check_eq! { parsed.physical.jump => 0, "unset physicals default to zero" }

        // TrainValue (C4InfoCore.cpp:279-285): zero stays zero, caps hold,
        // never decreases.
        let mut zero = 0;
        PhysicalInfo::train_value(&mut zero, 100, C4_MAX_PHYSICAL);
        check_eq! { zero => 0 }
        let mut value = 99_950;
        PhysicalInfo::train_value(&mut value, 100, C4_MAX_PHYSICAL);
        check_eq! { value => C4_MAX_PHYSICAL }
        let mut above = 120_000;
        PhysicalInfo::train_value(&mut above, 100, C4_MAX_PHYSICAL);
        check_eq! { above => 120_000, "never decreased by training" }
    }

    #[test]
    fn def_core_physical_after_an_intervening_section_is_ignored() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=GAP1
[Foo]
Energy=1
[Physical]
Energy=50000
Walk=35000
"#,
        )
        .expect("def core parses");

        check_eq! { parsed.physical => PhysicalInfo::default() }
    }

    #[test]
    fn def_core_physical_before_def_core_is_ignored() {
        let parsed = parse_def_core(
            br#"[Physical]
Energy=50000
Walk=35000
[DefCore]
id=PREV
"#,
        )
        .expect("def core parses");

        check_eq! { parsed.physical => PhysicalInfo::default() }
    }

    #[test]
    fn def_core_nested_physical_section_is_ignored() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=NEST
 [Physical]
 Energy=50000
 Walk=35000
"#,
        )
        .expect("def core parses");

        check_eq! { parsed.physical => PhysicalInfo::default() }
    }

    #[test]
    fn def_core_nested_intervening_section_does_not_block_physical_follow_name() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=CHLD
 [Foo]
 Energy=1
[Physical]
Energy=50000
"#,
        )
        .expect("def core parses");

        check_eq! { parsed.physical.energy => 50_000 }
    }

    #[test]
    fn def_core_physical_requires_exact_compiler_key_names() {
        let mismatched = parse_def_core(b"[DefCore]\nid=CASE\n[Physical]\nENERGY=50000\n")
            .expect("case-mismatched physical parses");
        check_eq! { mismatched.physical => PhysicalInfo::default() }

        let exact = parse_def_core(b"[DefCore]\nid=GOOD\n[Physical]\nEnergy=50000\n")
            .expect("well-formed physical parses");
        check_eq! { exact.physical.energy => 50_000 }
    }

    #[test]
    fn def_core_duplicate_values_and_sections_use_the_first_occurrence() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=DUPL
Mass=100
Mass=5
[Physical]
Walk=35000
[Physical]
Walk=1
Jump=25000
[DefCore]
Mass=7
Value=9
"#,
        )
        .expect("duplicate DefCore entries parse");

        check_eq! { parsed.id => "DUPL" }
        check_eq! { parsed.mass => 100 }
        check_eq! { parsed.physical.walk => 35_000 }
        check_eq! { parsed.physical.jump => 0 }
        check_eq! { parsed.value => 0 }
    }

    #[test]
    fn def_core_duplicate_mass_keeps_the_first_value() {
        let parsed = parse_def_core(b"[DefCore]\nid=DUPE\nMass=125\nMass=7\n")
            .expect("duplicate Mass entries parse");

        check_eq! { parsed.mass => 125 }
    }

    #[test]
    fn def_core_key_and_physical_section_names_are_case_sensitive() {
        let parsed = parse_def_core(
            br#"[DefCore]
id=CASE
mass=125
[physical]
Energy=50000
"#,
        )
        .expect("case-mismatched names remain inert");

        check_eq! { parsed.mass => 0, "lowercase mass is not the Mass field" }
        check_eq! { parsed.physical.energy => 0, "lowercase [physical] is not the [Physical] section" }
    }

    #[test]
    fn def_core_category_tokens_are_case_sensitive() {
        let parsed = parse_def_core(b"[DefCore]\nid=BITS\nCategory=C4D_Structure|c4d_goal\n")
            .expect("an unknown category token only warns");

        check_eq! { parsed.category => 1 << 1, "the mismatched token contributes no category bits" }
    }

    #[test]
    fn def_core_dollar_prefixed_numbers_use_field_defaults() {
        let parsed = parse_def_core(b"[DefCore]\nid=NUMS\nMass=$FF\nScale=$80\n")
            .expect("invalid numeric prefixes use field defaults");

        check_eq! { parsed.mass => 0 }
        check_eq! { parsed.graphics_scale => 100 }
    }

    #[test]
    fn parse_def_core_uses_create_name_tree_tokenization() {
        let parsed = parse_def_core(
            b"id=ROOT\n[DefCore] ; comment\nid=TOKN\nMASS=1000\nMass =100\n\
[Physical]\t# x\nEnergy=123\n[1Extra]\nJump=456\n[PHYSICAL]\nWalk=789\n",
        )
        .expect("tokenized DefCore parses");

        check_eq! { parsed.id => "TOKN", "pre-section id stays at the tree root" }
        check_eq! { parsed.mass => 0, "case and trailing key spaces are exact" }
        check_eq! { parsed.physical.energy => 123, "section trailing text is ignored" }
        check_eq! { parsed.physical.jump => 456, "non-alpha section lines are inert" }
        check_eq! { parsed.physical.walk => 0, "section names are case-sensitive" }

        check! { matches!(parse_def_core(b"id=XYZ1\nMass=100\n"), Err(DefinitionError::MissingDefCoreField("id"))) }
    }

    #[test]
    fn def_core_main_fields_follow_create_name_tree_hierarchy() {
        let parsed = parse_def_core(
            br#"[Container]
 [DefCore]
 id=NEST
 Mass=999
[DefCore] ; root sibling
id=ROOT
 [Nested]
 Mass=777
Mass=42
 [MalformedContext]
Broken
  Value=11
[Physical]
Energy=50000
"#,
        )
        .expect("hierarchical DefCore parses");

        check_eq! { parsed.id => "ROOT", "a nested DefCore must not shadow a later root sibling" }
        check_eq! { parsed.mass => 42, "an unindented value dedents from the nested section" }
        check_eq! { parsed.value => 11, "a malformed named line performs its native dedent before rejection" }
        check_eq! { parsed.physical.energy => 50_000, "the shared tree retains native FollowName adjacency" }

        let section_shadow = parse_def_core(b"[DefCore]\nid=SECT\n [Mass] 8\nMass=77\n")
            .expect("field-named section DefCore parses");
        check_eq! { section_shadow.mass => 8, "Name() selects the first matching child without checking node kind" }
    }

    #[test]
    fn parse_def_core_fire_fields() {
        // ContactIncinerate / NoBurnDecay / NoBurnDamage (C4Def fire fields;
        // ContactIncinerate feeds the CrossCheck Tick35 arm,
        // C4GameObjects.cpp:121-125).
        let data = br#"
            [DefCore]
            id=FIRY
            Name=Firy
            ContactIncinerate=10
            NoBurnDecay=1
            NoBreath=1
            NoBurnDamage=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.contact_incinerate => 10 }
        check! { parsed.no_burn_decay }
        check! { parsed.no_breath }
        check! { parsed.no_burn_damage }

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.contact_incinerate => 0, "default: not inflammable" }
        check! { !parsed.no_burn_decay }
        check! { !parsed.no_breath, "default: breathing" }
        check! { !parsed.no_burn_damage }
    }

    #[test]
    fn parse_def_core_closed_container_retains_mode() {
        let parsed = parse_def_core(
            br#"
                [DefCore]
                id=SAFE
                ClosedContainer=2
            "#,
        )
        .expect("closed container parses");
        check_eq! { parsed.closed_container => 2 }

        let open = parse_def_core(b"[DefCore]\nid=OPEN\n").expect("default parses");
        check_eq! { open.closed_container => 0 }
    }

    #[test]
    fn parse_def_core_blast_shield_and_horizontal_fix() {
        // ContainBlast=1 shields contents from explosions (the DoExplosion
        // container walk, C4Effect.cpp:884; C4Def.cpp:380) and
        // HorizontalFix=1 exempts a def from shockwave flings
        // (C4Def::NoHorizontalMove, C4Def.cpp:383; read back through
        // GetDefCoreVal by BlastObjectsShockwaveCheck).
        let data = br#"
            [DefCore]
            id=HUT1
            Name=Hut
            ContainBlast=1
            HorizontalFix=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.contain_blast => 1 }
        check_eq! { parsed.no_horizontal_move => 1 }

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.contain_blast => 0, "default: contents take blasts" }
        check_eq! { parsed.no_horizontal_move => 0, "default: movable" }
    }

    #[test]
    fn parse_def_core_blast_incinerate() {
        // BlastIncinerate=N: incinerate when accumulated Damage reaches N
        // after a blast (C4Def.cpp:315, default 0 = off; consumed by
        // C4Object::Blast, C4Object.cpp:1421-1423).
        let data = br#"
            [DefCore]
            id=TRE1
            Name=Tree
            BlastIncinerate=50
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.blast_incinerate => 50 }

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.blast_incinerate => 0, "default: no blast incinerate" }
    }

    #[test]
    fn parse_def_core_fire_top_and_default() {
        // C4Shape::CompileFunc compiles FireTop directly into DefCore with
        // default zero (C4Shape.cpp:496-510; C4Def.cpp:300-302).
        let parsed = parse_def_core(b"[DefCore]\nid=WMPF\nFireTop=10\n").expect("def core parses");
        check_eq! { parsed.fire_top => 10 }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("default def core parses");
        check_eq! { defaulted.fire_top => 0 }
    }

    #[test]
    fn parse_def_core_lift_top_and_default() {
        // C4Def.cpp:385 stores LiftTop as a signed DefCore integer.
        let parsed = parse_def_core(b"[DefCore]\nid=ELEV\nLiftTop=20\n").expect("def core parses");
        check_eq! { parsed.lift_top => 20 }

        let defaulted = parse_def_core(b"[DefCore]\nid=ELEV\n").expect("default def core parses");
        check_eq! { defaulted.lift_top => 0 }
    }

    #[test]
    fn parse_def_core_set_ocf_fields() {
        // The DefCore flags feeding C4Object::SetOCF (C4Object.cpp:526-666):
        // Entrance rect (C4Def.cpp:309), Exclusive (:313), Prey (:354),
        // Edible (:355), RotatedEntrance (:377), Chop -> Chopable (:378),
        // AttractLightning (:391), NoFight (:413).
        let data = br#"
            [DefCore]
            id=CSTL
            Name=Castle
            Entrance=-10,20,20,15
            Exclusive=1
            Prey=1
            Edible=1
            RotatedEntrance=45
            Chop=1
            AttractLightning=1
            NoFight=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.entrance => Some(PictureRect {x: -10, y: 20, width: 20, height: 15}) }
        check! { parsed.exclusive }
        check! { parsed.prey }
        check! { parsed.edible }
        check_eq! { parsed.rotated_entrance => 45 }
        check! { parsed.chopable }
        check! { parsed.attract_lightning }
        check! { parsed.no_fight }

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        check_eq! { parsed.entrance => None, "default: no entrance area" }
        check! { !parsed.exclusive }
        check! { !parsed.prey }
        check! { !parsed.edible }
        check_eq! { parsed.rotated_entrance => 0 }
        check! { !parsed.chopable }
        check! { !parsed.attract_lightning }
        check! { !parsed.no_fight }
    }

    #[test]
    fn parse_act_map_records_object_disabled() {
        // ObjectDisabled= (C4ActionDef::Disabled, C4Def.cpp:106): actions
        // that suspend the object — they veto OCF_Collection and
        // OCF_FightReady (SetOCF, C4Object.cpp:597,608).
        let data = br#"
[Action]
Name=Build
Procedure=Build
ObjectDisabled=1

[Action]
Name=Walk
Procedure=Walk
"#;
        let map = parse_act_map(data).expect("act map parsed");
        check! { map.get("Build").expect("build action").disabled }
        check! { !map.get("Walk").expect("walk action").disabled }
    }

    #[test]
    fn parse_act_map_preserves_complete_cpp_reflection_values() {
        let data = br#"
[Action]
Name=Reflect
Procedure=OddProcedure
Directions=-2
FlipDir=-3
Length=-4
Attach=-5
Delay=-6
Facet=1,,3,4,,6
FacetBase=2
FacetTopFace=-7
FacetTargetStretch=3
NextAction=Missing
NoOtherAction=2
StartCall=None
EndCall=End
AbortCall=none
PhaseCall=Phase
Sound=TravelSound
ObjectDisabled=-8
DigFree=-9
EnergyUsage=-10
InLiquidAction=Swim
TurnAction=Turn
Reverse=2
Step=-11

[Action]
Length=5

[Action]
Name=TextDefaults
Directions=false
Reverse=true
Step=false
Step=7
Attach=cnat_left
EnergyUsage=12junk
length=9
"#;
        let map = parse_act_map(data).expect("complete action table parses");
        let action = map.get("Reflect").expect("reflection action exists");
        check_eq! { map.get("").and_then(|action| action.length) => Some(5) }
        let text_defaults = map.get("TextDefaults").expect("text-default action exists");
        check_eq! { text_defaults.reflected_ints.get("Directions") => Some(&1) }
        check_eq! { text_defaults.reflected_ints.get("Reverse") => Some(&0) }
        check_eq! { text_defaults.reflected_ints.get("Step") => Some(&1) }
        check_eq! { text_defaults.reflected_ints.get("Attach") => Some(&0) }
        check_eq! { text_defaults.reflected_ints.get("EnergyUsage") => Some(&12) }
        check_eq! { text_defaults.length => None, "lower-case key is unknown" }
        check_eq! { action.procedure.as_deref() => Some("OddProcedure") }
        check_eq! { action.next_action.as_deref() => Some("Missing") }
        check_eq! { action.start_call => None, "CrossMap clears None callbacks" }
        check_eq! { action.abort_call => None, "CrossMap clears None case-insensitively" }
        check_eq! { action.end_call.as_deref() => Some("End") }
        check_eq! { action.phase_call.as_deref() => Some("Phase") }
        check_eq! { action.sound.as_deref() => Some("TravelSound") }
        check_eq! { action.in_liquid_action.as_deref() => Some("Swim") }
        check_eq! { action.turn_action.as_deref() => Some("Turn") }
        check_eq! { action.facet => Some(ActionFacet {x: 1, y: 0, width: 3, height: 4, target_x: 0, target_y: 6,}) }
        check! { action.no_other_action }
        check! { action.disabled }
        check! { action.facet_base }
        check! { action.facet_top_face }
        check! { action.facet_target_stretch }
        check! { action.reverse }
        check_eq! { action.directions => Some(-2) }
        check_eq! { action.flip_dir => Some(-3) }
        check_eq! { action.length => Some(-4) }
        check_eq! { action.delay => Some(-6) }
        check_eq! { action.step => Some(-11) }
        check_eq! { action.attach => (-5i32) as u32, "bit tests keep raw two's-complement bits" }
        for (entry, expected) in [
            ("Directions", -2),
            ("FlipDir", -3),
            ("Length", -4),
            ("Attach", -5),
            ("Delay", -6),
            ("FacetBase", 2),
            ("FacetTopFace", -7),
            ("FacetTargetStretch", 3),
            ("NoOtherAction", 2),
            ("ObjectDisabled", -8),
            ("DigFree", -9),
            ("EnergyUsage", -10),
            ("Reverse", 2),
            ("Step", -11),
        ] {
            check_eq! { action.reflected_ints.get(entry) => Some(&expected), "{entry}" }
        }
    }

    #[test]
    fn parse_act_map_records_dig_free() {
        let data = br#"
[Action]
Name=Dig
Procedure=Dig
DigFree=24
"#;
        let map = parse_act_map(data).expect("act map parsed");
        let dig = map.get("Dig").expect("dig action present");
        check_eq! { dig.dig_free => Some(24) }
    }

    #[test]
    fn parse_act_map_records_attach_mask() {
        let data = br#"
[Action]
Name=Scale
Procedure=Scale
Attach=CNAT_Left|CNAT_Bottom
"#;
        let map = parse_act_map(data).expect("act map parsed");
        let scale = map.get("Scale").expect("scale action present");
        check_eq! { scale.attach => 9 }
        check_eq! { scale.reflected_ints.get("Attach") => Some(&9) }
    }

    #[test]
    fn parse_act_map_accepts_trailing_section_comments_like_cpp() {
        let data = br#"
[Action] # ready, as used by shipped Airlock ActMaps
Name=Open
Length=7

[Action] trailing text is ignored by CreateNameTree
Name=Close
Length=9
"#;
        let map = parse_act_map(data).expect("commented action headers parse");
        check_eq! { map.get("Open").and_then(|action| action.length) => Some(7) }
        check_eq! { map.get("Close").and_then(|action| action.length) => Some(9) }
    }

    #[test]
    fn parse_act_map_accepts_cr_only_name_tree_lines() {
        let map = parse_act_map(
            b"[Action]\rName=First\rLength=7\rNextAction=Second\r[Action]\rName=Second\rLength=9\r",
        )
        .expect("CR-only ActMap parses");

        let first = map.get("First").expect("first action exists");
        check_eq! { first.length => Some(7) }
        check_eq! { first.next_action_index => 1 }
        check_eq! { map.get("Second").and_then(|action| action.length) => Some(9) }
    }

    #[test]
    fn parse_act_map_matches_cpp_slot_order_indentation_and_exact_keys() {
        let data = br#"# [comment slot]
[Other]
[Action]
Name =Wrong
Name=First
Length =9
Length=7
Delay	=5
[1]
Sound=[
[Other]
 [Action]
 Name=Nested
[Action]
Name=Second
Default=Ghost
"#;
        let map = parse_act_map(data).expect("C++ name-tree edge map parses");
        check_eq! { map.default_action => None, "Default is not an ActMap field" }
        check_eq! { map.actions.len() => 8, "every raw '[' allocates one slot" }
        check_eq! { map.actions[0].0 => "First" }
        check_eq! { map.actions[0].1.length => Some(7) }
        check_eq! { map.actions[0].1.delay => Some(5) }
        check_eq! { map.actions[0].1.sound.as_deref() => Some("[") }
        check_eq! { map.actions[1].0 => "Second" }
        check! { map.actions[2..].iter().all(|(name, _)| name.is_empty()) }
        check! { map.get("Nested").is_none(), "nested Action is not root" }
    }

    #[test]
    fn parse_act_map_preserves_native_string_bytes_and_stops_at_nul() {
        let mut data = b"[Action]\nName=Raw\nSound=".to_vec();
        data.extend(std::iter::repeat_n(b'a', 29));
        data.extend_from_slice("é".as_bytes());
        data.extend_from_slice(b"\nPhaseCall=");
        data.push(0xff);
        data.extend_from_slice(b"\0[Action]\nName=AfterNul\n");

        let map = parse_act_map(&data).expect("raw-byte ActMap parses");
        check_eq! { map.actions.len() => 1, "post-NUL brackets are invisible" }
        let raw = map.get("Raw").expect("pre-NUL action exists");
        let sound = clonk_script::c4_string_bytes(raw.sound.as_deref().expect("Sound retained"));
        check_eq! { sound => [vec![b'a'; 29], vec![0xc3]].concat() }
        check_eq! { clonk_script::c4_string_bytes(raw.phase_call.as_deref().expect("raw call retained")) => vec![0xff] }
        check! { map.get("AfterNul").is_none() }
    }

    #[test]
    fn action_int_and_attach_parsing_follow_stdcompiler_cursors() {
        check_eq! { parse_action_i32("-0x2") => Some(0) }
        check_eq! { parse_action_i32("+0x2") => Some(0) }
        check_eq! { parse_action_i32("0x10junk") => Some(16) }
        check_eq! { parse_action_i32("junk") => None }
        if std::mem::size_of::<std::os::raw::c_long>() == 8 {
            check_eq! { parse_action_i32("2147483648") => Some(i32::MIN) }
            check_eq! { parse_action_i32("0xFFFFFFFF") => Some(-1) }
            check_eq! { parse_action_i32("999999999999999999999999") => Some(-1) }
            check_eq! { parse_action_i32("-999999999999999999999999") => Some(0) }
        } else {
            check_eq! { parse_action_i32("2147483648") => Some(i32::MAX) }
            check_eq! { parse_action_i32("0xFFFFFFFF") => Some(i32::MAX) }
            check_eq! { parse_action_i32("999999999999999999999999") => Some(i32::MAX) }
            check_eq! { parse_action_i32("-999999999999999999999999") => Some(i32::MIN) }
        }

        for (source, expected) in [
            ("CNAT_Left # comment|CNAT_Bottom", 1),
            ("CNAT_Left,CNAT_Bottom", 1),
            ("Unknown_Name|CNAT_Bottom", 8),
            ("1junk|CNAT_Bottom", 1),
            ("CNAT_Left||CNAT_Bottom", 0),
            ("CNAT_Left|", 0),
            ("|CNAT_Left", 0),
            ("CNAT_Left|#comment", 0),
        ] {
            check_eq! { parse_action_attach(source) => expected, "{source}" }
        }

        let map =
            parse_act_map(b"[Action]\nName=Numbers\nLength=-0x2\nAttach=CNAT_Left # comment\n")
                .expect("numeric cursor integration parses");
        let action = map.get("Numbers").expect("numeric action exists");
        check_eq! { action.reflected_ints.get("Length") => Some(&0) }
        check_eq! { action.reflected_ints.get("Attach") => Some(&1) }
    }

    #[test]
    fn parse_def_core_value_mass_picture() {
        let data = br#"
            [DefCore]
            id=VALU
            Name=Valuable
            Value=75
            Mass=12
            Picture=1,2,32,24
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.value => 75 }
        check_eq! { parsed.mass => 12 }
        check_eq! { parsed.picture => Some(PictureRect {x: 1, y: 2, width: 32, height: 24}) }
    }

    #[test]
    fn parse_def_core_collection_fields() {
        let data = br#"
            [DefCore]
            id=PACK
            Width=20
            Height=40
            Offset=-10,-20
            Collection=-5,-10,10,20
            CollectionLimit=3
            Collectible=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.shape => Some(PictureRect {x: -10, y: -20, width: 20, height: 40}) }
        check_eq! { parsed.collection => Some(PictureRect {x: -5, y: -10, width: 10, height: 20}) }
        check_eq! { parsed.collection_limit => 3 }
        check! { parsed.collectible }
    }

    #[test]
    fn parse_def_core_fragile_uses_nonzero_truthiness_and_defaults_false() {
        let parsed =
            parse_def_core(b"[DefCore]\nid=BOOM\nFragile=-2\n").expect("Fragile DefCore parses");
        check! { parsed.fragile }

        let defaulted = parse_def_core(b"[DefCore]\nid=SAFE\n").expect("default DefCore parses");
        check! { !defaulted.fragile }
    }

    #[test]
    fn parse_def_core_projectile_preserves_signed_value_and_default() {
        let parsed = parse_def_core(b"[DefCore]\nid=ROCK\nProjectile=-2\n")
            .expect("Projectile DefCore parses");
        check_eq! { parsed.projectile => -2 }

        let defaulted = parse_def_core(b"[DefCore]\nid=SAFE\n").expect("default DefCore parses");
        check_eq! { defaulted.projectile => 0 }
    }

    #[test]
    fn parse_def_core_no_get_preserves_signed_value_and_default() {
        // C4DefCore::CompileFunc stores NoGet as an int32_t with default 0
        // (src/C4Def.cpp:412; src/C4Def.h:264). Menu code treats any
        // nonzero value as excluding the object from get/activate menus.
        let parsed =
            parse_def_core(b"[DefCore]\nid=LOCK\nNoGet=-2\n").expect("NoGet DefCore parses");
        check_eq! { parsed.no_get => -2 }

        let defaulted = parse_def_core(b"[DefCore]\nid=OPEN\n").expect("default DefCore parses");
        check_eq! { defaulted.no_get => 0 }
    }

    #[test]
    fn parse_def_core_grab_put_get_bitfield() {
        // StdBitfieldAdapt tokens (src/C4Def.cpp:364-373), e.g. the Lorry's
        // `GrabPutGet=C4D_GrabGet|C4D_GrabPut`.
        let data = br#"
            [DefCore]
            id=LORY
            GrabPutGet=C4D_GrabGet|C4D_GrabPut
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.grab_put_get => 3 }

        let data = br#"
            [DefCore]
            id=CONT
            GrabPutGet=C4D_GrabPut
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.grab_put_get => 1 }

        let get_only = parse_def_core(b"[DefCore]\nid=GETR\nGrabPutGet=C4D_GrabGet\n")
            .expect("get-only DefCore parses");
        check_eq! { get_only.grab_put_get => 2 }

        // Hazard's shipped SupplyBox uses the equivalent decimal form.
        let numeric = parse_def_core(b"[DefCore]\nid=SUPP\nGrabPutGet=3\n")
            .expect("numeric GrabPutGet parses");
        check_eq! { numeric.grab_put_get => 3 }

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("default DefCore parses");
        check_eq! { defaulted.grab_put_get => 0 }
    }

    #[test]
    fn parse_def_core_vehicle_control() {
        // Plain integer compile (src/C4Def.cpp:398), e.g. the Airship's
        // `VehicleControl=2` (C4D_VehicleControl_Inside).
        let data = br#"
            [DefCore]
            id=SHIP
            VehicleControl=2
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.vehicle_control => 2 }

        let data = br#"
            [DefCore]
            id=CONT
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.vehicle_control => 0, "default 0" }
    }

    #[test]
    fn parse_def_core_solid_mask_target_rect() {
        let data = br#"
            [DefCore]
            id=BASE
            Width=12
            Height=18
            Offset=-4,-6
            SolidMask=2,3,8,9,-1,4
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.solid_mask => Some(TargetRect {x: 2, y: 3, width: 8, height: 9, target_x: -1, target_y: 4,}) }
    }

    #[test]
    fn parse_def_core_top_face_target_rect() {
        // C4DefCore::CompileFunc reads TopFace as C4TargetRect
        // (src/C4Def.cpp:306); the last two values are the draw target.
        let data = br#"
            [DefCore]
            id=ELEC
            TopFace=0,1,24,26,-3,4
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.top_face => Some(TargetRect {x: 0, y: 1, width: 24, height: 26, target_x: -3, target_y: 4,}) }

        let defaulted = parse_def_core(b"[DefCore]\nid=ELEC\n").expect("defcore parsed");
        check_eq! { defaulted.top_face => None, "C4TargetRect defaults empty" }
    }

    #[test]
    fn parse_def_core_rotate_field() {
        let data = br#"
            [DefCore]
            id=SPNR
            Rotate=12
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.rotateable => 12 }
    }

    #[test]
    fn parse_def_core_stretch_growth_field_like_cpp() {
        // Mirrors src/C4Def.cpp:387: DefCore `StretchGrowth` compiles into
        // `GrowthType` with default 0. Hand-derived golden: explicit value 1 is
        // true, and an omitted field is false.
        let data = br#"
            [DefCore]
            id=GROW
            StretchGrowth=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check! { parsed.stretch_growth }

        let defaulted = parse_def_core(b"[DefCore]\nid=JOLT\n").expect("defcore parsed");
        check! { !defaulted.stretch_growth }
    }

    #[test]
    fn parse_def_core_oversize_field_like_cpp() {
        // C4Def.cpp:392: nonzero Oversize removes DoCon's upper FullCon
        // clamp, while an omitted field defaults to zero.
        let parsed = parse_def_core(b"[DefCore]\nid=GROW\nOversize=-2\n").expect("defcore parsed");
        check! { parsed.oversize, "every nonzero C++ BOOL value is true" }

        let defaulted = parse_def_core(b"[DefCore]\nid=JOLT\n").expect("defcore parsed");
        check! { !defaulted.oversize }
    }

    #[test]
    fn parse_def_core_rotated_solidmasks_flag_like_cpp() {
        // Mirrors src/C4Def.cpp:414: DefCore `RotatedSolidmasks` compiles
        // with default 0; nonzero lets C4Object::UpdateSolidMask keep the
        // mask while rotated (src/C4Object.cpp:5655).
        let data = br#"
            [DefCore]
            id=ELEV
            RotatedSolidmasks=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check! { parsed.rotated_solid_masks }

        let defaulted = parse_def_core(b"[DefCore]\nid=HUT0\n").expect("defcore parsed");
        check! { !defaulted.rotated_solid_masks }
    }

    #[test]
    fn parse_def_core_auto_context_menu_flag_like_cpp() {
        // Mirrors src/C4Def.cpp:416: DefCore `AutoContextMenu` compiles as
        // an integer flag.
        let parsed =
            parse_def_core(b"[DefCore]\nid=HUT3\nAutoContextMenu=1\n").expect("defcore parsed");

        check! { parsed.auto_context_menu }
    }

    #[test]
    fn parse_def_core_auto_context_menu_defaults_off_like_cpp() {
        // Mirrors the default argument in src/C4Def.cpp:416: an omitted
        // `AutoContextMenu` field compiles as zero.
        let parsed = parse_def_core(b"[DefCore]\nid=CLNK\n").expect("defcore parsed");

        check! { !parsed.auto_context_menu }
    }

    #[test]
    fn parse_def_core_silent_commands_integer_and_default_like_cpp() {
        // C4Def::CompileFunc reads the int32 SilentCommands with a zero
        // default (src/C4Def.cpp:404), not through the Boolean reader.
        let enabled =
            parse_def_core(b"[DefCore]\nid=CLNK\nSilentCommands=1\n").expect("defcore parsed");
        check! { enabled.silent_commands }

        let invalid =
            parse_def_core(b"[DefCore]\nid=CLNK\nSilentCommands=yes\n").expect("defcore parsed");
        check! { !invalid.silent_commands }

        let defaulted = parse_def_core(b"[DefCore]\nid=ROCK\n").expect("defcore parsed");
        check! { !defaulted.silent_commands }
    }

    #[test]
    fn parse_def_core_construct_to_as_build_turn_to_like_cpp() {
        // C4Def::CompileFunc exposes the BuildTurnTo field under the legacy
        // DefCore key `ConstructTo` (src/C4Def.cpp:361).
        let parsed =
            parse_def_core(b"[DefCore]\nid=SITE\nConstructTo=DONE\n").expect("defcore parsed");
        check_eq! { parsed.build_turn_to.as_deref() => Some("DONE") }

        let defaulted = parse_def_core(b"[DefCore]\nid=SITE\n").expect("defcore parsed");
        check! { defaulted.build_turn_to.is_none() }

        let none =
            parse_def_core(b"[DefCore]\nid=SITE\nConstructTo=NONE\n").expect("defcore parsed");
        check! { none.build_turn_to.is_none() }
    }

    #[test]
    fn def_core_turn_to_fields_use_c4id_adapt() {
        let literal = |bytes: [u8; 4]| u32::from_le_bytes(bytes) as usize;
        let cases = [
            ("", None),
            ("ASH", None),
            ("ASH1", Some(literal(*b"ASH1"))),
            ("ASH1tail", Some(literal(*b"ASH1"))),
            ("ABCD extra", Some(literal(*b"ABCD"))),
            (" \tABCDtail", Some(literal(*b"ABCD"))),
            ("\u{000c}ABCD", None),
            ("\u{00a0}ABCD", None),
            ("NONE", None),
            ("NONEtail", None),
            ("none", Some(literal(*b"none"))),
            ("0000", None),
            ("00000", None),
            ("1234tail", Some(1234)),
            ("ab_1tail", Some(literal(*b"ab_1"))),
            ("AB-Ctail", Some(literal(*b"AB-C"))),
            ("AB.C", None),
            ("AB CD", None),
        ];

        for (input, expected_raw) in cases {
            let source = format!("[DefCore]\nid=TEST\nBurnTo={input}\nConstructTo={input}\n");
            let parsed = parse_def_core(source.as_bytes()).expect("DefCore turn-to IDs parse");
            let burn_raw = parsed.burn_turn_to.as_deref().map(clonk_script::c4_id_raw);
            let construct_raw = parsed.build_turn_to.as_deref().map(clonk_script::c4_id_raw);

            check_eq! { burn_raw => expected_raw, "BurnTo={input}" }
            check_eq! { construct_raw => expected_raw, "ConstructTo={input}" }
        }

        let high_byte_after_four = parse_def_core(b"[DefCore]\nid=TEST\nBurnTo=FIRE\x80tail\n")
            .expect("a suffix after the fixed buffer is ignored");
        check_eq! { high_byte_after_four.burn_turn_to.as_deref() => Some("FIRE") }

        let high_byte_before_four = parse_def_core(b"[DefCore]\nid=TEST\nBurnTo=FIR\x80E\n")
            .expect("a non-identifier byte terminates the token");
        check! { high_byte_before_four.burn_turn_to.is_none() }
    }

    #[test]
    fn parse_def_core_base_sale_flags_like_cpp() {
        // C4Def::CompileFunc reads Rebuy with default 0 and BaseAutoSell
        // with a GOLD-specific default of 1 (src/C4Def.cpp:359,457).
        let explicit = parse_def_core(b"[DefCore]\nid=ORE1\nRebuy=1\nBaseAutoSell=1\n")
            .expect("defcore parsed");
        check! { explicit.rebuyable }
        check! { explicit.base_auto_sell }

        let gold = parse_def_core(b"[DefCore]\nid=GOLD\n").expect("defcore parsed");
        check! { !gold.rebuyable }
        check! { gold.base_auto_sell }

        let ordinary = parse_def_core(b"[DefCore]\nid=ROCK\n").expect("defcore parsed");
        check! { !ordinary.rebuyable }
        check! { !ordinary.base_auto_sell }
    }

    #[test]
    fn parse_def_core_shape_vertices_and_contact_metadata() {
        let data = br#"
            [DefCore]
            id=CLNK
            Width=16
            Height=32
            Offset=-8,-16
            Vertices=3
            VertexX=0,-4,4
            VertexY=9,3,3
            VertexCNAT=8,1,2
            VertexFriction=100,300,300
            ContactDensity=25
            ContactCalls=1
            BorderBound=7
            UprightAttach=8
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.vertices.len() => 3 }
        check_eq! { parsed.vertices[0] => DefVertex {x: 0, y: 9, cnat: 8, friction: 100,} }
        check_eq! { parsed.vertices[2] => DefVertex {x: 4, y: 3, cnat: 2, friction: 300,} }
        check_eq! { parsed.contact_density => 25 }
        check! { parsed.contact_function_calls }
        check_eq! { parsed.border_bound => 7 }
        check_eq! { parsed.upright_attach => 8 }
    }

    #[test]
    fn parse_def_core_vertex_arrays_zero_fill_missing_entries() {
        let data = br#"
            [DefCore]
            id=SPRS
            Vertices=4
            VertexX=-2,2
            VertexY=5
            VertexFriction=20,30
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        check_eq! { parsed.vertices.len() => 4 }
        check_eq! { parsed.vertices[0].x => -2 }
        check_eq! { parsed.vertices[1].x => 2 }
        check_eq! { parsed.vertices[2].x => 0 }
        check_eq! { parsed.vertices[0].y => 5 }
        check_eq! { parsed.vertices[1].y => 0 }
        check_eq! { parsed.vertices[0].friction => 20 }
        check_eq! { parsed.vertices[1].friction => 30 }
        check_eq! { parsed.vertices[2].friction => 0 }
    }

    #[test]
    fn parse_def_core_retains_dormant_vertex_slots_beyond_count() {
        // C4Shape::CompileFunc adapts all C4D_MaxVertex array slots even
        // when VtxNum is smaller (src/C4Shape.cpp:496-509). A later
        // AddVertex overwrites only X/Y, so this dormant CNAT/friction must
        // survive definition loading (src/C4Shape.cpp:26-31).
        let parsed = parse_def_core(
            b"[DefCore]\nid=TABB\nVertices=1\nVertexX=3,30\nVertexY=4,40\n\
              VertexCNAT=8,10\nVertexFriction=100,250\n",
        )
        .expect("defcore parsed");

        check_eq! { parsed.vertices.len() => 1 }
        check_eq! { parsed.vertex_slots[1] => DefVertex {x: 30, y: 40, cnat: 10, friction: 250,} }
        check_eq! { parsed.vertex_slots.len() => C4D_MAX_VERTEX }
    }

    #[test]
    fn parse_def_core_components_list() {
        let data = br#"
            [DefCore]
            id=HUTS
            Components=WOOD=2;METL=1;ROCK;ZERO=0;NEGA=-3
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(
            parsed.components,
            [
                DefComponent {
                    id: "WOOD".to_string(),
                    count: 2,
                },
                DefComponent {
                    id: "METL".to_string(),
                    count: 1,
                },
                DefComponent {
                    id: "ROCK".to_string(),
                    count: 0,
                },
                DefComponent {
                    id: "ZERO".to_string(),
                    count: 0,
                },
                DefComponent {
                    id: "NEGA".to_string(),
                    count: -3,
                },
            ]
        );
    }

    #[test]
    fn c4id_lists_use_cpp_separators_cursor_and_stop_on_invalid_id() {
        let component = |id: &str, count| DefComponent {
            id: id.to_string(),
            count,
        };
        for (source, expected) in [
            ("ROCK=abc", vec![component("ROCK", 0)]),
            ("LOAM=2;bad;WOOD=3", vec![component("LOAM", 2)]),
            ("rock=5", vec![]),
            ("Rock=1;LOAM=2", vec![]),
            ("ROCK=1,LOAM=2", vec![component("ROCK", 1)]),
            ("ROCK=x;", vec![component("ROCK", 0)]),
            ("ROCKS=2", vec![component("ROCK", 0)]),
            ("ROCK:2;LOAM=3", vec![component("ROCK", 0)]),
            ("ROCK=12junk;LOAM=3", vec![component("ROCK", 12)]),
            ("ROCK=x;LOAM=2", vec![component("ROCK", 0)]),
            (
                "ROCK=;LOAM=2",
                vec![component("ROCK", 0), component("LOAM", 2)],
            ),
            ("ROCK=1;;LOAM=2", vec![component("ROCK", 1)]),
            ("ROCK=1;Rock=2;LOAM=3", vec![component("ROCK", 1)]),
            ("NONE=1;ROCK=2", vec![]),
            ("0000=1;ROCK=2", vec![]),
        ] {
            check_eq! { parse_components(source) => expected, "{source}" }
        }

        check! { parse_id_list("Rock").is_empty() }
        check_eq! { parse_id_list("REQ1;REQ2") => ["REQ1", "REQ2"] }
        for source in ["REQ1,REQ2", "REQ1 REQ2", "REQ1;Rock;REQ2", "REQ1=7;REQ2"] {
            check_eq! { parse_id_list(source) => ["REQ1"], "{source}" }
        }
    }

    #[test]
    fn parse_def_core_c4id_lists_skip_only_ascii_space_and_tab() {
        let accepted =
            parse_def_core(b"[DefCore]\nid=WSPC\nRequireDef= \tREQ1\nComponents=\t ROCK=2\n")
                .expect("space/tab-prefixed C4ID lists parse");
        check_eq! { accepted.require_defs => ["REQ1"] }
        check_eq! { accepted.components => [DefComponent {id: "ROCK".to_string(), count: 2,}] }

        for prefix in ['\u{000b}', '\u{000c}', '\u{00a0}'] {
            let source =
                format!("[DefCore]\nid=WSPC\nRequireDef={prefix}REQ1\nComponents={prefix}ROCK=2\n");
            let parsed = parse_def_core(source.as_bytes()).expect("DefCore parses");
            check! { parsed.require_defs.is_empty(), "prefix {prefix:?}" }
            check! { parsed.components.is_empty(), "prefix {prefix:?}" }
        }
    }

    #[test]
    fn definition_script_does_not_recurse_into_subgroups() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Nested.ocd");
        fs::create_dir(&def_dir).unwrap();
        write_fixture! { def_dir.join("DefCore.txt") => br#"[DefCore]
id=NNNN
Name=Nested
Category=C4D_Object
"# };
        write_fixture! { def_dir.join("Script.c") => b"func Root() {}" };
        let script_dir = def_dir.join("Helpers");
        fs::create_dir(&script_dir).unwrap();
        write_fixture! { script_dir.join("ScriptUS.c") => b"func NestedLocalized() {}" };
        write_fixture! { script_dir.join("Other.c") => b"func NestedOther() {}" };

        let group = Group::open(&def_dir).unwrap();
        let definition =
            Definition::load_with_languages(&group, &["US"]).expect("definition load succeeds");
        let paths = definition
            .script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("Script.c")] }
        check_eq! { definition.script.combined() => "\nfunc Root() {}" }
        check! { !definition.script.combined().contains("NestedLocalized") }
        check! { !definition.script.combined().contains("NestedOther") }
    }

    #[test]
    fn definition_script_selects_fixed_components_in_cpp_order() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Scripts.ocd");
        fs::create_dir(&def_dir).unwrap();
        write_fixture! { def_dir.join("DefCore.txt") => b"[DefCore]\nid=SCRP\n" };
        write_fixture! { def_dir.join("Script.c") => b"func Base() {}" };
        write_fixture! { def_dir.join("ScriptUS.c") => b"func Localized() {}\n" };
        write_fixture! { def_dir.join("ScriptDE.c") => b"func German() {}" };
        write_fixture! { def_dir.join("C4ScriptUS.c") => b"func Legacy() {}" };
        write_fixture! { def_dir.join("ScriptOld.c") => b"func Obsolete() {}" };
        write_fixture! { def_dir.join("Other.c") => b"func Other() {}" };

        let definition =
            Definition::load_with_languages(&Group::open(&def_dir).unwrap(), &["US", "DE"])
                .expect("definition load succeeds");
        let paths = definition
            .script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("Script.c"), Path::new("ScriptUS.c"), Path::new("C4ScriptUS.c"),] }
        check_eq! { definition.script.combined() => "\nfunc Base() {}\nfunc Localized() {}\n\nfunc Legacy() {}" }
        check! { !definition.script.combined().contains("//#file") }
        check! { !definition.script.combined().contains("German") }
        check! { !definition.script.combined().contains("Obsolete") }
        check! { !definition.script.combined().contains("Other") }
    }

    #[test]
    fn definition_script_restarts_language_order_for_each_segment() {
        let temp = tempdir().unwrap();
        write_fixture! { temp.path().join("ScriptDE.c") => b"func German() {}" };
        write_fixture! { temp.path().join("C4ScriptUS.c") => b"func LegacyUS() {}" };

        let script = load_scripts(
            &Group::open(temp.path()).unwrap(),
            &["US-extra", "DE-extra"],
        )
        .expect("script components load");
        let paths = script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("ScriptDE.c"), Path::new("C4ScriptUS.c")] }
        check_eq! { script.combined() => "\nfunc German() {}\nfunc LegacyUS() {}" }
    }

    #[test]
    fn definition_script_loads_c4script_localization_without_script_component() {
        let temp = tempdir().unwrap();
        write_fixture! { temp.path().join("C4ScriptUS.c") => b"func LegacyOnly() {}" };

        let script = load_scripts(&Group::open(temp.path()).unwrap(), &["US"])
            .expect("script component loads");
        check_eq! { script.files().len() => 1 }
        check_eq! { script.files()[0].path => Path::new("C4ScriptUS.c") }
        check_eq! { script.combined() => "\nfunc LegacyOnly() {}" }
    }

    #[test]
    fn definition_script_empty_language_sequence_uses_one_empty_cpp_segment() {
        let temp = tempdir().unwrap();
        write_fixture! { temp.path().join("Script.c") => b"func Base() {}" };
        write_fixture! { temp.path().join("C4Script.c") => b"func Legacy() {}" };
        let languages: [&str; 0] = [];

        let script = load_scripts(&Group::open(temp.path()).unwrap(), &languages)
            .expect("script components load");
        let paths = script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("Script.c"), Path::new("Script.c"), Path::new("C4Script.c"),] }
        check_eq! { script.combined() => "\nfunc Base() {}\nfunc Base() {}\nfunc Legacy() {}" }
    }

    #[test]
    fn shipped_map_screen_excludes_obsolete_script_old() {
        let directory = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/Hazard.c4d/Structural.c4d/Deco.c4d/Screens.c4d/MapScreen.c4d"
        ));
        check! { directory.is_dir(), "the initialized official content submodule must provide {}", directory.display() }

        let definition = Definition::load_with_languages(
            &Group::open(directory).expect("open shipped MapScreen definition"),
            &["US", "DE"],
        )
        .expect("load shipped MapScreen definition");
        let paths = definition
            .script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("Script.c")] }
        check! { definition.script.combined().contains("Initialized") }
        check! { definition.script.combined().contains("MAP_MasterScreen") }
        check! { !definition.script.combined().contains("InitScreens") }
        check! { !definition.script.combined().contains("FxMapTimer") }
    }

    #[test]
    fn script_component_nul_truncates_only_its_own_file() {
        let temp = tempdir().unwrap();
        write_fixture! { temp.path().join("Script.c") => b"func Before() {}\0func Hidden() {}" };
        write_fixture! { temp.path().join("ScriptUS.c") => b"func After() {}\n" };

        let group = Group::open(temp.path()).unwrap();
        let script = load_scripts(&group, &["US"]).expect("script components load");
        let combined = clonk_script::c4_string_bytes(script.combined());

        check_eq! { combined => b"\nfunc Before() {}\nfunc After() {}\n" }
        check! { combined.windows(b"func Before() {}".len()).any(|window| window == b"func Before() {}") }
        check! { !combined.windows(b"func Hidden() {}".len()).any(|window| window == b"func Hidden() {}") }
        check! { combined.windows(b"func After() {}".len()).any(|window| window == b"func After() {}") }
        check! { !combined.contains(&0) }
    }

    #[test]
    fn unreadable_definition_script_candidate_falls_through_to_next_language() {
        let bad_us = vec![b'x'; 64];
        let mut packed = crate::MutableGroup::new("unreadable-script.bin");
        packed
            .add_file(
                "DefCore.txt",
                b"[DefCore]\nid=FALL\nCategory=C4D_Object\n".to_vec(),
            )
            .unwrap();
        packed
            .add_file(
                "ScriptDE.c",
                b"func Fallback() {}\0func Hidden() {}".to_vec(),
            )
            .unwrap();
        packed
            .add_file("C4ScriptUS.c", b"func Legacy() {}".to_vec())
            .unwrap();
        packed.add_file("ScriptUS.c", bad_us.clone()).unwrap();

        let mut raw = packed.pack_raw().unwrap();
        raw.truncate(raw.len() - bad_us.len());
        let group = Group::from_memory(
            PathBuf::from("unreadable-script.c4d"),
            crate::compress_c4group_image(&raw).unwrap(),
        )
        .unwrap();
        check! { group.exists("ScriptUS.c") }
        check! { group.read_file("ScriptUS.c").is_err() }
        check_eq! { group.read_file("ScriptDE.c").unwrap() => b"func Fallback() {}\0func Hidden() {}" }

        let definition =
            Definition::load_with_languages(&group, &["US", "DE"]).expect("fallback loads");
        let paths = definition
            .script
            .files()
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();
        check_eq! { paths => vec![Path::new("ScriptDE.c"), Path::new("C4ScriptUS.c")] }
        check_eq! { definition.script.combined() => "\nfunc Fallback() {}\nfunc Legacy() {}" }

        let no_fallback =
            Definition::load_with_languages(&group, &["US"]).expect("failed segment is optional");
        check_eq! { no_fallback.script.files().len() => 1 }
        check_eq! { no_fallback.script.files()[0].path => Path::new("C4ScriptUS.c") }
        check_eq! { no_fallback.script.combined() => "\nfunc Legacy() {}" }
    }

    #[test]
    fn readable_empty_definition_script_candidate_blocks_language_fallback() {
        let temp = tempdir().unwrap();
        write_fixture! { temp.path().join("ScriptUS.c") => [] };
        write_fixture! { temp.path().join("ScriptDE.c") => b"func MustNotLoad() {}" };

        let script = load_scripts(&Group::open(temp.path()).unwrap(), &["US", "DE"])
            .expect("empty candidate loads");
        check_eq! { script.files().len() => 1 }
        check_eq! { script.files()[0].path => Path::new("ScriptUS.c") }
        check_eq! { script.combined() => "\n" }
    }

    #[test]
    fn load_definition_ignores_nested_definitions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Parent.ocd");
        fs::create_dir(&def_dir).unwrap();
        write_fixture! { def_dir.join("DefCore.txt") => br#"[DefCore]
id=PARA
Name=Parent
Category=C4D_Object
"# };
        write_fixture! { def_dir.join("Script.c") => b"func Parent() {}\n" };
        let nested = def_dir.join("Child.ocd");
        fs::create_dir(&nested).unwrap();
        write_fixture! { nested.join("DefCore.txt") => br#"[DefCore]
id=CHLD
Name=Child
Category=C4D_Object
"# };
        write_fixture! { nested.join("Script.c") => b"func Child() {}\n" };

        let group = Group::open(&def_dir).unwrap();
        let definition = Definition::load(&group).expect("definition load succeeds");
        check_eq! { definition.script.files.len() => 1 }
        check_eq! { definition.script.files[0].path => PathBuf::from("Script.c") }
    }
}
