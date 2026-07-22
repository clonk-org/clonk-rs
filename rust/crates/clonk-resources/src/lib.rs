pub mod bitmap;
pub mod definition;
pub mod font;
pub mod graphics;
pub mod group;
pub mod group_writer;
pub mod language;
mod legacy_paths;
pub mod material;
pub mod network_scenario;
pub mod particle;
pub mod rtf;
pub mod scenario;
mod script_strings;
pub mod texmap;

pub use definition::{
    ActionDefinition, ActionMap, ColorByOwnerMask, DefComponent, DefCore,
    Definition as ResourceDefinition, DefinitionError, DefinitionScript, DefinitionScriptFile,
    PhysicalInfo, PictureRect, RankExtensionFormatError, RankNameTable, C4_MAX_PHYSICAL,
};
pub use font::{
    load_endeavour_font, load_font_definitions, load_ttf, select_font_definition, FontCatalog,
    FontDefinition, FontResource, FontResourceError, FontRole, ResolvedFontSpec,
};
pub use graphics::{GraphicsError, GraphicsImage, GraphicsResource};
pub use group::{Group, GroupEntry, GroupError};
pub use group_writer::{
    c4group_file_crc, compress_c4group_image, MutableGroup, MutableGroupChildMut,
    MutableGroupEntryKind, MutableGroupError,
};
pub use language::{ComponentGroups, LanguageInfo, LanguagePacks, LoadedComponent};
pub use legacy_paths::{path_from_legacy_bytes, path_to_legacy_bytes};
pub use material::{MaterialDefinition, MaterialError, MaterialLibrary};
pub use network_scenario::{
    combine_network_scenario, merge_extracted_group_entries, NetworkScenarioError,
};
pub use particle::{
    ParticleDefinition, ParticleDefinitionCore, ParticleDefinitionError, ParticleFacet,
};
pub use scenario::{
    discover, discover_many, discover_many_with_languages, discover_many_with_languages_and_packs,
    discover_many_with_languages_and_packs_with_progress, discover_with_languages,
    discover_with_languages_and_packs, discover_with_languages_and_packs_with_progress,
    ScenarioDiscoveryError, ScenarioDiscoveryProgress, ScenarioEntry, ScenarioEntryKind,
};
pub use script_strings::{
    decode_legacy_script_text, decode_legacy_system_text, encode_legacy_script_text,
    localize_script_source, localize_script_source_with_components,
};
