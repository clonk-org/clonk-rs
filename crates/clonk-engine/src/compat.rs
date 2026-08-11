use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{hash_map::Entry, BTreeMap, HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::rc::Rc;

use crate::action::{ScriptCallbackTarget, SharedActionLibrary};
use crate::command::{
    definition_id_to_c4id, AcquireScriptResult, CallResultAction, CommandData,
    CommandDefinitionSnapshot, CommandEvent, CommandEventInstanceKind, CommandFailureFeedback,
    CommandFailureReason, CommandId, CommandMode, CommandObjectSnapshot, CommandObjectSnapshots,
    CommandOperation, CommandPlayerSnapshot, CommandRequest, CommandRuntimeContext, CommandStack,
    CommandStackSnapshot, CommandView, MAX_COMMAND_STACK,
};
use crate::effect::{EffectCommand, EffectState, EffectVarValue};
use crate::material::MaterialSet;
use crate::math::{
    fixed10, fixed100, fixtoi, fixtoi_prec, integer_distance, itofix, itofix_prec, C4Fixed,
    FixedVec2,
};
use crate::message::{
    MessageCommand, MessageKind, MessageSpec, ALIGNMENT_FLAGS, FLAG_DROP_SPEECH,
    HORIZONTAL_POSITION_FLAGS, MESSAGE_ANY_OWNER, MESSAGE_NO_OWNER, VERTICAL_POSITION_FLAGS,
};
use crate::object::log_runtime_call_frames;
use crate::ocf;
use crate::rng::LcgRng;
use crate::scenario::{ScenarioValue, ScenarioValueStore};
use crate::scoreboard::ScoreboardPresentationSink;
use crate::sector::{SectorMap, SectorObject};
use crate::sky::SkyAdjustment;
use crate::text_spec::{parse_text_spec, TextSpec};
use crate::transfer::TransferZoneTable;
use crate::{
    encode_bridge_action_data, ActionProcedure, ActionState, ActionUpdate, AudioCommand,
    ChangeDefContentsSort, CommandDirection, CrewInfoCoreFields, CrewInfoLink, CrewObjectInfo,
    CrewPermanentPortrait, CrewPortrait, CrewPortraitState, CrewSelectionState, DefinitionId,
    DefinitionRect, Direction, DrawTransform, EnvironmentSettings, FloatVector2,
    GraphicsOverlayMode, Landscape, MenuRequest, MenuRequestKind, ObjectBaseGraphics,
    ObjectGraphicsOverlay, ObjectId, ObjectState, ObjectStatus, ObjectUpdate, ObjectVertex,
    ParticleCommand, ParticleConfig, ParticleLayer, ParticleScope, PathFinder,
    PathfinderDebugSnapshot, PauseGameRequest, PhysicalsUpdate, PhysicsSettings,
    PlayerControlState, PlayerState, QueuedCommand, RgbColor, ScoreboardState, ShapeAttachRecord,
    ShapeVertexBuffer, SpawnConfig, SpeechFallback, TeamConfiguration, TeamInfo,
    TransferZoneCommand, TransferZoneRect, TransferZoneState, Vector2, C4D_BORDER_BOTTOM,
    C4D_BORDER_LAYER, C4D_BORDER_SIDES, C4D_BORDER_TOP, CATEGORY_SORT_LIMIT, CNAT_BOTTOM,
    CNAT_CENTER, CNAT_LEFT, CNAT_NO_COLLISION, CNAT_RIGHT, CNAT_TOP, DEFAULT_CATEGORY,
    DEFAULT_MUSIC_LEVEL, FULL_CON, OWNER_NONE,
};
#[cfg(test)]
use crate::{
    LiquidSegment, PlayerViewport, CONTENTS_SCOPE_GROWTH_VISITS, FIND_CANDIDATE_ENUM_NANOS,
    FIND_CANDIDATE_MATCH_NANOS, FIND_CONDITION_OBJECT_REFRESHES, FIND_CRITERION_PARSE_NANOS,
    FORCE_LEGACY_FIND_FUNC_SCALAR_PREFIX,
};
use chrono::{Datelike, Local, Timelike};
use clonk_resources::{PhysicalInfo, RankNameTable};
use clonk_script::{
    C4VType, Engine as ScriptEngine, HostCallArg, HostRegistrationSnapshot, RuntimeError, Value,
    ValueMap,
};
use std::mem;
use std::sync::Arc;
use tracing::{debug, error, info};

mod commands;
mod contexts;
mod effects;
mod landscape;
mod menus_messages;
mod object_state;
pub(crate) mod objects;
mod players;
mod registration;
mod sounds;
mod values;
pub(crate) mod world;

pub(crate) use commands::*;
pub use contexts::*;
pub(crate) use effects::*;
pub use landscape::*;
pub(crate) use menus_messages::*;
pub(crate) use object_state::*;
pub use objects::*;
pub use players::*;
pub use registration::*;
pub use sounds::*;
pub use values::*;
pub use world::*;

thread_local! {
    static HOST_CONTEXT: RefCell<Option<EffectHostContext>> = const { RefCell::new(None) };
    static RANDOM_CONTEXT: RefCell<Option<Rc<RandomContext>>> = const { RefCell::new(None) };
    // C++ SafeRandom is a process-global, deliberately unsynchronized
    // libc-rand stream (C4Random.h:35,71-75). Keep presentation-only script
    // choices off the lockstep RandomContext just as the oracle does.
    static SCRIPT_SAFE_RNG: RefCell<crate::particles::SafeRng> =
        RefCell::new(crate::particles::SafeRng::default());
    static ENVIRONMENT_CONTEXT: RefCell<Option<Rc<EnvironmentContext>>> = const {
        RefCell::new(None)
    };
    static PHYSICS_CONTEXT: RefCell<Option<Rc<PhysicsContext>>> = const {
        RefCell::new(None)
    };
    static AUDIO_CONTEXT: RefCell<Option<AudioRegistry>> = const { RefCell::new(None) };
    // GetFairCrewPhysical runs while an object physical is being resolved.
    // Keep the definition-only GetID/GetPhysical surfaces available without
    // recursively borrowing the active object host context.
    static FAIR_CREW_DEFINITION_CONTEXT: RefCell<Option<(DefinitionId, PhysicalInfo)>> =
        const { RefCell::new(None) };
}

thread_local! {
    /// C4ObjectMenu::CloseQuerying (C4ObjectMenu.h:64): the per-menu
    /// recursion check for the MenuQueryCancel callback — shared between
    /// the host-fn close path and the engine-side close hooks.
    static CLOSE_QUERYING: RefCell<std::collections::HashSet<ObjectId>> =
        RefCell::new(std::collections::HashSet::new());
}
const LEGACY_GAME_PALETTE: &[u8; 256 * 3] = include_bytes!("../../../planet/Graphics.c4g/C4.PAL");

/// Run `f` against the installed host context, yielding `fallback` when the
/// engine has none installed. C++ host functions always execute inside a
/// context; the `Option` models the window between frames, where every
/// wrapper returns its own inert value rather than touching engine state.
fn with_host_context<R>(fallback: R, f: impl FnOnce(&EffectHostContext) -> R) -> R {
    HOST_CONTEXT.with(|cell| cell.borrow().as_ref().map_or(fallback, f))
}

/// `with_host_context` for the wrappers that mutate engine state.
fn with_host_context_mut<R>(fallback: R, f: impl FnOnce(&mut EffectHostContext) -> R) -> R {
    HOST_CONTEXT.with(|cell| cell.borrow_mut().as_mut().map_or(fallback, f))
}

/// `with_host_context` for the wrappers that raise a script error instead of
/// returning an inert value when no context is installed. `missing` is only
/// turned into a `RuntimeError` on that failing path.
fn try_with_host_context<T>(
    missing: &str,
    f: impl FnOnce(&EffectHostContext) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        f(cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new(missing))?)
    })
}

/// `try_with_host_context` for the wrappers that mutate engine state.
fn try_with_host_context_mut<T>(
    missing: &str,
    f: impl FnOnce(&mut EffectHostContext) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        f(cell
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| RuntimeError::new(missing))?)
    })
}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
// Runtime host contexts intentionally share single-threaded script engines by Arc identity.
mod tests {
    use super::*;
    use crate::command::{CommandId, CommandOperation};
    use crate::message::{FLAG_BOTTOM, FLAG_LEFT, FLAG_WIDTH_REL, FLAG_X_REL};
    use crate::ocf;
    use crate::ActionLibrary;
    use crate::ActionSpec;
    use crate::AudioCommand;
    use clonk_resources::C4_MAX_PHYSICAL;
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::fmt;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{subscriber, Level};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    // The C++ host-compat battery stays one module so its ids remain
    // `compat::tests::*`; the bodies live in byte-verbatim contiguous parts.
    include!("compat/tests/part_01.rs");
    include!("compat/tests/part_02.rs");
    include!("compat/tests/part_03.rs");
    include!("compat/tests/part_04.rs");
    include!("compat/tests/part_05.rs");
    include!("compat/tests/part_06.rs");
    include!("compat/tests/part_07.rs");
    include!("compat/tests/part_08.rs");
    include!("compat/tests/part_09.rs");
    include!("compat/tests/part_10.rs");
    include!("compat/tests/part_11.rs");
}
