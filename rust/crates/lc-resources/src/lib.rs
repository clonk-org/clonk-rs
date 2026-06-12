pub mod bitmap;
pub mod definition;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod font;
pub mod graphics;
pub mod group;
pub mod material;
pub mod scenario;
pub mod texmap;

pub use definition::{
    ActionDefinition, ActionMap, ColorByOwnerMask, DefComponent, DefCore,
    Definition as ResourceDefinition, DefinitionError, DefinitionScript, DefinitionScriptFile,
    PhysicalInfo, PictureRect, C4_MAX_PHYSICAL,
};
pub use font::{load_endeavour_font, load_ttf, FontResource, FontResourceError};
pub use graphics::{GraphicsError, GraphicsImage, GraphicsResource};
pub use group::{Group, GroupEntry, GroupError};
pub use material::{MaterialDefinition, MaterialError, MaterialLibrary};
pub use scenario::{
    discover, discover_many, ScenarioDiscoveryError, ScenarioEntry, ScenarioEntryKind,
};
