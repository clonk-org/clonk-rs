pub mod definition;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod font;
pub mod graphics;
pub mod group;
pub mod material;
pub mod scenario;

pub use definition::{
    ActionDefinition, ActionMap, ColorByOwnerMask, DefCore, Definition as ResourceDefinition,
    DefinitionError, DefinitionScript, DefinitionScriptFile, PictureRect,
};
pub use font::{load_endeavour_font, load_ttf, FontResource, FontResourceError};
pub use graphics::{GraphicsError, GraphicsImage, GraphicsResource};
pub use group::{Group, GroupEntry, GroupError};
pub use material::{MaterialDefinition, MaterialError, MaterialLibrary};
pub use scenario::{
    discover, discover_many, ScenarioDiscoveryError, ScenarioEntry, ScenarioEntryKind,
};
