// These tests were written under clonk-engine's crate-level lint config (lib.rs
// top); mirror it here so the byte-identical code lints the same as it did
// inline, rather than surfacing style lints the move would otherwise expose.
#![allow(dead_code, unreachable_patterns, unused_variables)]
#![allow(
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::match_like_matches_macro,
    clippy::needless_range_loop,
    clippy::question_mark,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::vec_init_then_push
)]

// clonk-engine's former inline unit tests (lib.rs `mod tests`). Kept in their own
// test-only package (not folded into tests/it) so this large harness can compile
// at a lower optimization level while clonk-engine remains fully optimized. A
// test-only edit here also recompiles just this leaf crate — measured ~10s ->
// ~2s versus editing the inline module, which dirtied lib.rs and cascaded
// through every downstream crate. The glob re-export lets the module body keep
// its original `use super::*;`; the submodule `pub use`s below cover items the
// glob (root-only) doesn't reach.
#[allow(unused_imports)]
pub use clonk_engine::action::DEFAULT_ACTION_NAME;
#[allow(unused_imports)]
pub use clonk_engine::command::{
    AcquireScriptResult, CommandData, CommandId, CommandMode, CommandRequest,
};
#[allow(unused_imports)]
pub use clonk_engine::compat::{object_reference_value, NestedObjectOutcome};
#[allow(unused_imports)]
pub use clonk_engine::compat::{LandscapeOperation, ObjectOrderCommand, PlayerCommand};
#[allow(unused_imports)]
pub use clonk_engine::effect::EffectCommand;
#[allow(unused_imports)]
pub use clonk_engine::math::{fixed100, fixtoi, itofix, FixedVec2};
#[allow(unused_imports)]
pub use clonk_engine::sector::{SectorKey, SECTOR_HEIGHT};
#[allow(unused_imports)]
pub use clonk_engine::transfer::{TransferZoneCommand, TransferZoneRect};
#[allow(unused_imports)]
pub use clonk_engine::*;
#[allow(unused_imports)]
pub use clonk_resources::{ResourceDefinition as ResourceDefinitionData, C4_MAX_PHYSICAL};
#[allow(unused_imports)]
pub use clonk_script::DebuggerHooks;

mod tests {
    use super::*;

    // Area part files spliced into this same `tests` module: each part is a
    // bare item sequence (not a child module), so test ids stay `tests::<fn>`.
    include!("parts/fire_blast.rs");
    include!("parts/materials_pxs.rs");
    include!("parts/action_procedures.rs");
    include!("parts/find_funcs.rs");
    include!("parts/menus_commands.rs");
    include!("parts/physicals_seasons.rs");
    include!("parts/players_crew.rs");
    include!("parts/changedef_layers.rs");
    include!("parts/crewinfo_contents.rs");
    include!("parts/effects.rs");
    include!("parts/solidmask_shape.rs");
    include!("parts/ocf_rotation.rs");
    include!("parts/order_exec.rs");
}
