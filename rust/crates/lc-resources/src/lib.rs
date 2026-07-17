pub mod bitmap;
pub mod definition;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod font;
pub mod graphics;
pub mod group;
pub mod group_writer;
pub mod language;
pub mod material;
pub mod network_scenario;
pub mod rtf;
pub mod scenario;
mod script_strings;
pub mod texmap;

pub use definition::{
    ActionDefinition, ActionMap, ColorByOwnerMask, DefComponent, DefCore,
    Definition as ResourceDefinition, DefinitionError, DefinitionScript, DefinitionScriptFile,
    PhysicalInfo, PictureRect, C4_MAX_PHYSICAL,
};
pub use font::{
    load_endeavour_font, load_font_definitions, load_ttf, select_font_definition, FontCatalog,
    FontDefinition, FontResource, FontResourceError, FontRole, ResolvedFontSpec,
};
pub use graphics::{GraphicsError, GraphicsImage, GraphicsResource};
pub use group::{Group, GroupEntry, GroupError};
pub use group_writer::{c4group_file_crc, MutableGroup, MutableGroupChildMut, MutableGroupError};
pub use language::{ComponentGroups, LanguageInfo, LanguagePacks, LoadedComponent};
pub use material::{MaterialDefinition, MaterialError, MaterialLibrary};
pub use network_scenario::{combine_network_scenario, NetworkScenarioError};
pub use scenario::{
    discover, discover_many, discover_many_with_languages,
    discover_many_with_languages_and_packs, discover_with_languages,
    discover_with_languages_and_packs, ScenarioDiscoveryError, ScenarioEntry, ScenarioEntryKind,
};
pub use script_strings::{
    decode_legacy_script_text, localize_script_source,
    localize_script_source_with_components,
};
