use std::cell::RefCell;
use std::collections::{hash_map::Entry, BTreeMap, HashMap, HashSet};
use std::convert::TryFrom;
use std::rc::Rc;

use crate::command::{
    CommandData, CommandId, CommandMode, CommandOperation, CommandRequest, MAX_COMMAND_STACK,
};
use crate::effect::{EffectCommand, EffectState, EffectVarValue};
use crate::material::MaterialSet;
use crate::math::{fixed100, fixtoi_prec, integer_distance, itofix, itofix_prec, C4Fixed, FixedVec2};
use crate::message::{
    MessageCommand, MessageKind, MessageSpec, ALIGNMENT_FLAGS, FLAG_MULTIPLE,
    HORIZONTAL_POSITION_FLAGS, VERTICAL_POSITION_FLAGS,
};
use crate::ocf;
use crate::rng::LcgRng;
use crate::sector::{SectorMap, SectorObject};
#[cfg(test)]
use crate::LiquidSegment;
#[cfg(test)]
use crate::PlayerViewport;
use crate::{
    encode_bridge_action_data, ActionLibrary, ActionProcedure, ActionUpdate, AudioCommand,
    CommandDirection, CrewSelectionState, DefinitionId, DefinitionRect, Direction, DrawTransform,
    EnvironmentSettings, FloatVector2, GraphicsOverlayMode, Landscape, ObjectBaseGraphics,
    ObjectGraphicsOverlay, ObjectId, ObjectState, ObjectStatus, ObjectUpdate, ObjectVertex,
    ParticleCommand, ParticleConfig, ParticleLayer, ParticleScope, PathFinder, PhysicalsUpdate,
    PhysicsSettings, PlayerState, QueuedCommand, SpawnConfig, TransferZoneCommand,
    TransferZoneRect, TransferZoneState, Vector2, CATEGORY_SORT_LIMIT, CNAT_BOTTOM, CNAT_CENTER,
    CNAT_LEFT, CNAT_NO_COLLISION, CNAT_RIGHT, CNAT_TOP, DEFAULT_CATEGORY, FULL_CON, OWNER_NONE,
};
use lc_resources::PhysicalInfo;
use lc_script::{Engine as ScriptEngine, RuntimeError, Value};
use std::mem;
use std::sync::Arc;
use tracing::{debug, info};

thread_local! {
    static SETACTION_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static HOST_CONTEXT: RefCell<Option<EffectHostContext>> = const { RefCell::new(None) };
    static RANDOM_CONTEXT: RefCell<Option<Rc<RandomContext>>> = const { RefCell::new(None) };
    static ENVIRONMENT_CONTEXT: RefCell<Option<Rc<EnvironmentContext>>> = const {
        RefCell::new(None)
    };
    static PHYSICS_CONTEXT: RefCell<Option<Rc<PhysicsContext>>> = const {
        RefCell::new(None)
    };
    static AUDIO_CONTEXT: RefCell<Option<AudioRegistry>> = const { RefCell::new(None) };
}

const OWNER_ANY: i32 = -2;
const MATERIAL_NONE: i32 = -1;
const ANY_CONTAINER_SENTINEL: i32 = 123;
const NO_CONTAINER_SENTINEL: i32 = 124;
const MAX_VERTEX_COUNT: i32 = 30;
const C4V_ANY: i32 = 0;
const C4V_INT: i32 = 1;
const C4V_BOOL: i32 = 2;
const C4V_ID: i32 = 3;
const C4V_OBJECT: i32 = 4;
const C4V_STRING: i32 = 5;
const C4V_ARRAY: i32 = 6;
const C4V_MAP: i32 = 7;
const LEGACY_MAX_ARRAY_SIZE: i32 = 1_000_000;

#[derive(Debug, Clone)]
pub(crate) struct HostWorldObject {
    pub id: ObjectId,
    definition_id: DefinitionId,
    status: ObjectStatus,
    alive: bool,
    /// C4Object::InLiquid (the cached flag FnInLiquid reads).
    in_liquid: bool,
    pub action_name: String,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub action_procedure: Option<String>,
    pub owner: i32,
    pub category: i32,
    pub energy: i32,
    pub construction: i32,
    #[allow(dead_code)]
    pub damage: i32,
    pub ocf: u32,
    pub position: Vector2,
    #[allow(dead_code)]
    pub velocity: Vector2,
    pub rotation: i32,
    pub vertices: Vec<ObjectVertex>,
    #[allow(dead_code)]
    pub action_data: i32,
    pub action_ticks: u32,
    pub action_phase: i32,
    container: Option<ObjectId>,
    contents: Vec<ObjectId>,
    #[allow(dead_code)]
    pub draw_transform: Option<DrawTransform>,
    /// C++ CommandName strings of the object's command stack, top first
    /// (FnGetCommand, C4Script.cpp:918-945). A frame-start snapshot —
    /// mid-frame command changes are not re-read (C++ reads live).
    pub command_names: Vec<String>,
    /// Full object-state snapshot for nested script calls (Find_Func,
    /// GameCall): lets host functions build a complete object scope for
    /// another object mid-VM-call. `None` in legacy fixture contexts.
    state: Option<Rc<ObjectState>>,
    /// C4Object::LastEnergyLossCausePlayer (kill trace) — carried beside
    /// the state snapshot because it lives on the engine object wrapper.
    pub last_energy_loss_cause: i32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DefinitionMetadata {
    /// DefCore `Name` (FnGetName's def form, C4Script.cpp:992-1005).
    pub name: String,
    pub category: i32,
    pub ocf_base: u32,
    pub crew_member: bool,
    /// ActMap for building nested object scopes (Find_Func targets).
    pub action_library: ActionLibrary,
    #[allow(dead_code)]
    pub value: i32,
    #[allow(dead_code)]
    pub mass: i32,
    pub constructable: bool,
    pub shape: Option<DefinitionRect>,
    pub construction_offset: i32,
    #[allow(dead_code)]
    pub basement: i32,
    /// The `[Physical]` section (GetPhysical's def form, C4Script.cpp:652).
    pub physical: PhysicalInfo,
    /// DefCore `Components` in list order (C4IDList; GetComponent's
    /// count/index forms, C4Script.cpp:2685-2709).
    pub components: Vec<(String, u32)>,
    /// DefCore `LineConnect` bits (C4D_Power_Consumer etc.;
    /// FnEnergyCheck, C4Script.cpp:1845-1856).
    pub line_connect: u32,
    /// ClonkNames newline count (C4ObjectInfoList::New's name draw range,
    /// C4InfoCore.cpp:411); None = use the game standard names.
    pub clonk_name_newlines: Option<i32>,
    /// DefCore StretchGrowth (the con-scaling mode; DoCon's bottom
    /// adjust shape math).
    pub stretch_growth: bool,
    /// DefCore Line type (C4D_Line*; nonzero skips con-scaling and the
    /// DoCon bottom adjust — C4Object::UpdateShape's early return).
    pub line: i32,
    /// The definition's shape vertices (full-Con). Pending-spawn preview
    /// scopes seed from these so creation callbacks (AdjustSeatVertex,
    /// CHBM Connect) mutate the REAL vertex list, not an empty one.
    pub vertices: Vec<ObjectVertex>,
}

/// `SetPhysical`/`GetPhysical` modes (C4Script.cpp:552-555).
const PHYS_CURRENT: i32 = 0;
const PHYS_PERMANENT: i32 = 1;
const PHYS_TEMPORARY: i32 = 2;
const PHYS_STACK_TEMPORARY: i32 = 3;

#[derive(Debug, Clone)]
pub(crate) enum PlayerCommand {
    AdjustHomeBaseMaterial {
        player_id: i32,
        definition_id: DefinitionId,
        delta: i32,
    },
    AdjustHomeBaseProduction {
        player_id: i32,
        definition_id: DefinitionId,
        delta: i32,
    },
    GrantKnowledge {
        player_id: i32,
        definition_id: DefinitionId,
    },
    RevokeKnowledge {
        player_id: i32,
        definition_id: DefinitionId,
    },
    /// `FnSetWealth` (C4Script.cpp:2761-2766), already clamped.
    SetWealth { player_id: i32, value: i32 },
    /// FnSetCursor (C4Script.cpp:2951-2958): pPlr->SetCursor(pObj) and,
    /// unless fNoSelectCrew, SelectCrew(pObj, true).
    SetCursor {
        player_id: i32,
        object: Option<ObjectId>,
        select_crew: bool,
    },
}

impl HostWorldObject {
    #[cfg(test)]
    pub(crate) fn new(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        energy: i32,
        construction: i32,
        position: Vector2,
        velocity: Vector2,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: u32,
        container: Option<ObjectId>,
    ) -> Self {
        Self::with_category(
            id,
            definition_id,
            status,
            action_name,
            action_target,
            action_target2,
            action_procedure,
            owner,
            DEFAULT_CATEGORY,
            energy,
            construction,
            0,
            position,
            velocity,
            0,
            vertices,
            action_data,
            action_ticks,
            0,
            container,
            None,
        )
    }

    pub(crate) fn with_category(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        category: i32,
        energy: i32,
        construction: i32,
        damage: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: u32,
        action_phase: i32,
        container: Option<ObjectId>,
        draw_transform: Option<DrawTransform>,
    ) -> Self {
        Self {
            id,
            definition_id: definition_id.into(),
            status,
            alive: true,
            in_liquid: false,
            action_name: action_name.into(),
            action_target,
            action_target2,
            action_procedure,
            owner,
            category,
            energy,
            construction: construction.clamp(0, FULL_CON),
            damage,
            ocf: ocf::NORMAL,
            position,
            velocity,
            rotation,
            vertices,
            action_data,
            action_ticks,
            action_phase,
            container,
            contents: Vec::new(),
            draw_transform,
            command_names: Vec::new(),
            state: None,
            last_energy_loss_cause: OWNER_NONE,
        }
    }

    pub(crate) fn with_command_names(mut self, command_names: Vec<String>) -> Self {
        self.command_names = command_names;
        self
    }

    pub(crate) fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub(crate) fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = in_liquid;
        self
    }

    pub(crate) fn with_ocf(mut self, ocf: u32) -> Self {
        self.ocf = ocf;
        self
    }

    pub(crate) fn with_full_state(mut self, state: Rc<ObjectState>) -> Self {
        self.state = Some(state);
        self
    }

    pub(crate) fn with_last_energy_loss_cause(mut self, cause: i32) -> Self {
        self.last_energy_loss_cause = cause;
        self
    }

    /// The full state snapshot, when the context was built by the engine
    /// (`Engine::host_world_context`). See the `state` field docs.
    pub(crate) fn full_state(&self) -> Option<&Rc<ObjectState>> {
        self.state.as_ref()
    }

    pub fn alive(&self) -> bool {
        self.alive
    }

    pub fn in_liquid(&self) -> bool {
        self.in_liquid
    }

    pub fn definition_id(&self) -> &str {
        &self.definition_id
    }

    pub fn status(&self) -> ObjectStatus {
        self.status
    }

    pub fn ocf(&self) -> u32 {
        self.ocf
    }

    pub fn action_target(&self, index: usize) -> Option<ObjectId> {
        match index {
            0 => self.action_target,
            1 => self.action_target2,
            _ => None,
        }
    }

    pub fn procedure_name(&self) -> Option<&str> {
        self.action_procedure.as_deref()
    }

    pub fn owner(&self) -> i32 {
        self.owner
    }

    /// C4Object::Controller — carried on the full-state snapshot; legacy
    /// fixture snapshots without one fall back to the owner (the Init
    /// default, C4Object.cpp:162).
    pub fn controller(&self) -> i32 {
        self.state
            .as_ref()
            .map(|state| state.controller)
            .unwrap_or(self.owner)
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn energy(&self) -> i32 {
        self.energy
    }

    pub fn construction(&self) -> i32 {
        self.construction
    }

    #[allow(dead_code)]
    pub fn damage(&self) -> i32 {
        self.damage
    }

    pub fn action_name(&self) -> &str {
        &self.action_name
    }

    pub fn container(&self) -> Option<ObjectId> {
        self.container
    }

    pub fn contents(&self) -> &[ObjectId] {
        &self.contents
    }

    pub fn with_contents(mut self, contents: Vec<ObjectId>) -> Self {
        self.contents = contents;
        self
    }

    pub fn is_present(&self) -> bool {
        !matches!(self.status, ObjectStatus::Deleted)
    }

    pub fn position(&self) -> Vector2 {
        self.position
    }

    pub fn velocity(&self) -> Vector2 {
        self.velocity
    }

    pub fn vertices(&self) -> &[ObjectVertex] {
        &self.vertices
    }

    pub fn action_ticks(&self) -> u32 {
        self.action_ticks
    }

    #[allow(dead_code)]
    pub fn action_data(&self) -> i32 {
        self.action_data
    }

    pub fn action_phase(&self) -> i32 {
        self.action_phase
    }

    pub fn set_action_phase(&mut self, phase: i32) {
        self.action_phase = phase;
    }
}

// Not `derive(Debug)`: `ScriptEngine` (in `definition_scripts`) has no Debug.
#[derive(Clone)]
pub(crate) struct HostWorldContext {
    objects: Rc<HashMap<ObjectId, HostWorldObject>>,
    order: Rc<Vec<ObjectId>>,
    landscape: Option<Rc<Landscape>>,
    definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
    /// Lazily built on the first sector query: most callbacks never run
    /// one, and an eager build per host context made every tick quadratic
    /// in the object count.
    sectors: RefCell<Option<Rc<SectorMap>>>,
    transfer_zones: Rc<Vec<TransferZoneState>>,
    players: Rc<HashMap<i32, PlayerState>>,
    player_order: Rc<Vec<i32>>,
    crew_selection: Rc<HashMap<i32, CrewSelectionState>>,
    next_object_id: u64,
    team_home_base_rule: bool,
    /// C4RULE_StructuresNeedEnergy (Game.Rules; FnEnergyCheck gates on
    /// it, C4Script.cpp:1845-1856).
    structures_need_energy: bool,
    /// Game.Names newline count (the New() fallback name range).
    standard_name_newlines: i32,
    /// Idle crew infos per (player, definition) — C4ObjectInfoList::GetIdle
    /// hits skip the New name draw (C4Player.cpp:1186-1195). Consumed
    /// entries decrement so several MakeCrewMember calls in one script
    /// call keep drawing like C++.
    idle_crew_counts: RefCell<HashMap<(i32, String), u32>>,
    /// Names of loaded particle defs (C4ParticleSystem::GetDef,
    /// C4Particles.cpp:465-473). `None` = no registry attached (legacy
    /// fixture contexts): name lookups behave permissively. `Some` = engine
    /// attached its registry: unknown names make the particle host functions
    /// return false exactly like the C++ GetDef-failure paths
    /// (C4Script.cpp:4874,4893,4917,4932).
    particle_defs: Option<Rc<std::collections::HashSet<String>>>,
    /// Compiled definition scripts, shared from `Engine.definitions`, so host
    /// functions can run script functions on other objects mid-VM-call
    /// (Find_Func/Sort_Func, GameCall). Empty in legacy fixture contexts.
    definition_scripts: Rc<HashMap<DefinitionId, Arc<ScriptEngine>>>,
    /// The material table (Game.Material): name lookups for FnMaterial
    /// (C4Script.cpp:2488-2491). `None` in legacy fixture contexts.
    materials: Option<Rc<MaterialSet>>,
    /// Crew object ranks from the engine's crew infos (`pObj->Info->Rank`;
    /// GetHiRank reads them, C4Player.cpp:1012). Objects without an entry
    /// behave like info-less crew (rank -1).
    crew_ranks: Rc<HashMap<u64, i32>>,
    /// The scenario script, shared from `Engine.scenario_script`, for
    /// GameCall/GameCallEx mid-VM-call resolution (C++ resolves on
    /// `Game.Script`, C4Script.cpp:3483). `None` when no scenario script is
    /// installed (and in fixture contexts).
    scenario_script: Option<Arc<ScriptEngine>>,
}

impl Default for HostWorldContext {
    fn default() -> Self {
        Self {
            objects: Rc::new(HashMap::new()),
            order: Rc::new(Vec::new()),
            landscape: None,
            definitions: Rc::new(HashMap::new()),
            sectors: RefCell::new(None),
            transfer_zones: Rc::new(Vec::new()),
            players: Rc::new(HashMap::new()),
            player_order: Rc::new(Vec::new()),
            crew_selection: Rc::new(HashMap::new()),
            next_object_id: 1,
            structures_need_energy: false,
            standard_name_newlines: 0,
            idle_crew_counts: RefCell::new(HashMap::new()),
            team_home_base_rule: false,
            particle_defs: None,
            definition_scripts: Rc::new(HashMap::new()),
            scenario_script: None,
            crew_ranks: Rc::new(HashMap::new()),
            materials: None,
        }
    }
}

impl HostWorldContext {
    pub(crate) fn with_structures_need_energy(mut self, value: bool) -> Self {
        self.structures_need_energy = value;
        self
    }

    pub(crate) fn with_crew_name_sources(
        mut self,
        standard_name_newlines: i32,
        idle_crew_counts: HashMap<(i32, String), u32>,
    ) -> Self {
        self.standard_name_newlines = standard_name_newlines;
        self.idle_crew_counts = RefCell::new(idle_crew_counts);
        self
    }

    pub(crate) fn structures_need_energy(&self) -> bool {
        self.structures_need_energy
    }

    #[cfg(test)]
    pub(crate) fn from_objects<I>(objects: I) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_objects_with_players<I, P>(objects: I, players: P) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
        P: IntoIterator<Item = PlayerState>,
    {
        let map = players
            .into_iter()
            .map(|state| (state.id, state))
            .collect::<HashMap<_, _>>();
        Self::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            map,
            HashMap::new(),
            1,
            false,
        )
    }

    pub(crate) fn with_landscape<I>(
        objects: I,
        landscape: Option<Landscape>,
        definitions: HashMap<DefinitionId, DefinitionMetadata>,
        transfer_zones: Vec<TransferZoneState>,
        players: HashMap<i32, PlayerState>,
        crew_selection: HashMap<i32, CrewSelectionState>,
        next_object_id: u64,
        team_home_base_rule: bool,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape_shared(
            objects,
            landscape,
            Rc::new(definitions),
            transfer_zones,
            players,
            crew_selection,
            next_object_id,
            team_home_base_rule,
        )
    }

    /// `with_landscape` with an already-shared metadata table: definitions
    /// are immutable during play, so the engine caches the table instead of
    /// re-cloning every ActionLibrary per host context.
    pub(crate) fn with_landscape_shared<I>(
        objects: I,
        landscape: Option<Landscape>,
        definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
        transfer_zones: Vec<TransferZoneState>,
        players: HashMap<i32, PlayerState>,
        crew_selection: HashMap<i32, CrewSelectionState>,
        next_object_id: u64,
        team_home_base_rule: bool,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        let map = objects.into_iter().collect::<Vec<HostWorldObject>>();
        let sectors = RefCell::new(None);
        let mut order = Vec::with_capacity(map.len());
        let mut lookup = HashMap::with_capacity(map.len());
        for object in map {
            let id = object.id;
            order.push(id);
            lookup.insert(id, object);
        }
        Self {
            objects: Rc::new(lookup),
            order: Rc::new(order),
            landscape: landscape.map(Rc::new),
            definitions,
            sectors,
            transfer_zones: Rc::new(transfer_zones),
            player_order: Rc::new({
                let mut ids: Vec<_> = players.keys().copied().collect();
                ids.sort_unstable();
                ids
            }),
            players: Rc::new(players),
            crew_selection: Rc::new(crew_selection),
            next_object_id,
            team_home_base_rule,
            structures_need_energy: false,
            standard_name_newlines: 0,
            idle_crew_counts: RefCell::new(HashMap::new()),
            particle_defs: None,
            definition_scripts: Rc::new(HashMap::new()),
            scenario_script: None,
            crew_ranks: Rc::new(HashMap::new()),
            materials: None,
        }
    }

    /// Attach the scenario script for GameCall/GameCallEx resolution.
    pub(crate) fn with_scenario_script(mut self, script: Option<Arc<ScriptEngine>>) -> Self {
        self.scenario_script = script;
        self
    }

    pub(crate) fn scenario_script(&self) -> Option<&Arc<ScriptEngine>> {
        self.scenario_script.as_ref()
    }

    /// Attach the engine's compiled definition scripts for nested script
    /// calls. See the `definition_scripts` field docs.
    pub(crate) fn with_definition_metadata(
        mut self,
        definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
    ) -> Self {
        self.definitions = definitions;
        self
    }

    pub(crate) fn with_definition_scripts(
        mut self,
        scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
    ) -> Self {
        self.definition_scripts = Rc::new(scripts);
        self
    }

    pub(crate) fn definition_script(&self, id: &str) -> Option<&Arc<ScriptEngine>> {
        self.definition_scripts.get(id)
    }

    /// Whether any definition script, global script, or host function knows
    /// `name` — the global-function-map lookup of `GetFirstFunc`
    /// (C4Aul.cpp:545-552).
    pub(crate) fn script_function_known(&self, name: &str) -> bool {
        self.definition_scripts.values().any(|script| {
            script.has_function(name)
                || script.has_global_function(name)
                || script.has_host_function(name)
        })
    }

    /// Attach the engine's particle def registry (names from
    /// `C4ParticleSystem` defs). See the `particle_defs` field docs.
    pub(crate) fn with_particle_defs(mut self, defs: std::collections::HashSet<String>) -> Self {
        self.particle_defs = Some(Rc::new(defs));
        self
    }

    /// Attach the material table (FnMaterial name lookups).
    pub(crate) fn with_materials(mut self, materials: Option<Rc<MaterialSet>>) -> Self {
        self.materials = materials;
        self
    }

    pub(crate) fn materials(&self) -> Option<&MaterialSet> {
        self.materials.as_deref()
    }

    /// Attach the engine's crew-info rank table (see `crew_ranks` docs).
    pub(crate) fn with_crew_ranks(mut self, ranks: Rc<HashMap<u64, i32>>) -> Self {
        self.crew_ranks = ranks;
        self
    }

    /// The crew object's Info rank; `None` for info-less objects.
    pub(crate) fn crew_rank(&self, object: u64) -> Option<i32> {
        self.crew_ranks.get(&object).copied()
    }

    /// `Some(known?)` when a registry is attached, `None` otherwise.
    pub(crate) fn particle_def_known(&self, name: &str) -> Option<bool> {
        self.particle_defs.as_ref().map(|defs| defs.contains(name))
    }

    /// `C4Id2Def` visibility: `Some(known?)` when the engine attached a
    /// definition table, `None` for legacy fixture contexts (empty table
    /// stays permissive like particle_def_known).
    pub(crate) fn definition_known(&self, id: &str) -> Option<bool> {
        (!self.definitions.is_empty()).then(|| self.definitions.contains_key(id))
    }

    pub(crate) fn get(&self, id: ObjectId) -> Option<&HostWorldObject> {
        self.objects.get(&id)
    }

    pub(crate) fn object_ids(&self) -> &[ObjectId] {
        self.order.as_ref().as_slice()
    }

    pub(crate) fn landscape_ref(&self) -> Option<&Landscape> {
        self.landscape.as_deref()
    }

    pub(crate) fn transfer_zones(&self) -> &[TransferZoneState] {
        self.transfer_zones.as_ref()
    }

    pub(crate) fn next_object_id(&self) -> u64 {
        self.next_object_id
    }

    pub(crate) fn with_next_object_id(mut self, next_object_id: u64) -> Self {
        self.next_object_id = next_object_id;
        self
    }

    pub(crate) fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    pub(crate) fn definition_category(&self, id: &str) -> Option<i32> {
        self.definitions.get(id).map(|meta| meta.category)
    }

    pub(crate) fn definition_metadata(&self, id: &str) -> Option<&DefinitionMetadata> {
        self.definitions.get(id)
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        host_object_shape_rect(object, &self.definitions)
    }

    /// The sector map over this context's objects, built on first use.
    fn sector_map(&self) -> Option<Rc<SectorMap>> {
        let landscape = self.landscape.as_ref()?;
        let mut cache = self.sectors.borrow_mut();
        if cache.is_none() {
            *cache = Some(Rc::new(build_host_sector_map(
                self.order.iter().filter_map(|id| self.objects.get(id)),
                &self.definitions,
                landscape,
            )));
        }
        cache.clone()
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.object_ids_in_area(&area)
        })
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.shape_ids_in_area(&area)
        })
    }

    pub(crate) fn player_ids(&self) -> &[i32] {
        self.player_order.as_ref()
    }

    pub(crate) fn player(&self, id: i32) -> Option<&PlayerState> {
        self.players.get(&id)
    }

    pub(crate) fn crew_selection(&self, id: i32) -> Option<&CrewSelectionState> {
        self.crew_selection.get(&id)
    }
}

fn build_host_sector_map<'a, I>(
    objects: I,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
    landscape: &Landscape,
) -> SectorMap
where
    I: IntoIterator<Item = &'a HostWorldObject>,
{
    let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
    let height = landscape.estimated_height();
    let mut sectors = SectorMap::new(width, height);
    sectors.rebuild(
        objects
            .into_iter()
            .filter_map(|object| host_sector_record(object, definitions)),
    );
    sectors
}

fn host_sector_record(
    object: &HostWorldObject,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
) -> Option<SectorObject> {
    if matches!(object.status(), ObjectStatus::Deleted) {
        return None;
    }
    Some(SectorObject {
        id: object.id,
        position: object.position(),
        shape_rect: host_object_shape_rect(object, definitions),
    })
}

fn host_object_shape_rect(
    object: &HostWorldObject,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
) -> DefinitionRect {
    definitions
        .get(object.definition_id())
        .and_then(|metadata| metadata.shape)
        .map(|rect| {
            DefinitionRect::new(
                object.position.x.saturating_add(rect.x),
                object.position.y.saturating_add(rect.y),
                rect.width,
                rect.height,
            )
        })
        .or_else(|| host_vertex_bounds_rect(object.position(), object.vertices()))
        .unwrap_or_else(|| DefinitionRect::new(object.position.x, object.position.y, 1, 1))
}

fn host_vertex_bounds_rect(position: Vector2, vertices: &[ObjectVertex]) -> Option<DefinitionRect> {
    let first = vertices.first()?;
    let mut min_x = first.x;
    let mut max_x = first.x;
    let mut min_y = first.y;
    let mut max_y = first.y;
    for vertex in &vertices[1..] {
        min_x = min_x.min(vertex.x);
        max_x = max_x.max(vertex.x);
        min_y = min_y.min(vertex.y);
        max_y = max_y.max(vertex.y);
    }
    Some(DefinitionRect::new(
        position.x.saturating_add(min_x),
        position.y.saturating_add(min_y),
        max_x.saturating_sub(min_x).saturating_add(1),
        max_y.saturating_sub(min_y).saturating_add(1),
    ))
}

trait WorldAccessor {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject>;
    fn object_ids(&self) -> Vec<ObjectId>;
    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect;
    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>>;
    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>>;
    /// Definition mass/value for the C4SO_Mass/C4SO_Value sorts.
    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata>;
    /// Whether any definition script (or host function) knows `name` —
    /// the `Game.ScriptEngine.GetFirstFunc` lookup C4FindObjectFunc does at
    /// construction (C4Aul.cpp:545-552).
    fn script_function_known(&self, name: &str) -> bool;
}

impl WorldAccessor for HostWorldContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get(id).cloned()
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.object_ids().to_vec()
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        self.object_shape_rect(object)
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.object_sector_ids_in_rect(rect)
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.shape_sector_ids_in_rect(rect)
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        HostWorldContext::definition_metadata(self, id).cloned()
    }

    fn script_function_known(&self, name: &str) -> bool {
        HostWorldContext::script_function_known(self, name)
    }
}

/// A borrow-free world view for Func-criterion searches: condition checks
/// read this clone while the nested-call seam re-borrows the live
/// HOST_CONTEXT per candidate. Snapshot semantics (mid-search mutations and
/// callback spawns are not re-read) are part of the documented copy-in/
/// copy-out divergence.
struct FuncFindView {
    world: HostWorldContext,
    pending_objects: HashMap<ObjectId, HostWorldObject>,
    pending_order: Vec<ObjectId>,
}

impl WorldAccessor for FuncFindView {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.pending_objects
            .get(&id)
            .cloned()
            .or_else(|| self.world.get(id).cloned())
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        let mut ids = self.world.object_ids().to_vec();
        ids.extend(self.pending_order.iter().copied());
        ids
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        if self.pending_objects.contains_key(&object.id) {
            host_object_shape_rect(object, self.world.definitions.as_ref())
        } else {
            self.world.object_shape_rect(object)
        }
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.object_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.pending_objects.get(&id) else {
                continue;
            };
            if rect.contains_point(object.position.x, object.position.y) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.shape_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.pending_objects.get(&id) else {
                continue;
            };
            if self.object_shape_rect(object).overlaps(&rect) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        HostWorldContext::definition_metadata(&self.world, id).cloned()
    }

    fn script_function_known(&self, name: &str) -> bool {
        self.world.script_function_known(name)
    }
}

/// Clones the active context's world view for a Func-criterion search.
fn snapshot_func_find_view() -> Option<FuncFindView> {
    HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().map(|context| FuncFindView {
            world: context.world.clone(),
            pending_objects: context.pending_objects.clone(),
            pending_order: context.pending_order.clone(),
        })
    })
}

/// Drops candidates a Func callback destroyed — the C++ Status re-checks
/// after `Check` (Find: C4FindObject.cpp:186-199; FindMany pre-sort erase:
/// C4FindObject.cpp:217-218).
fn retain_live_nested(ids: &mut Vec<ObjectId>) {
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow().as_ref() {
            ids.retain(|id| !context.nested_object_destroyed(*id));
        }
    });
}

impl WorldAccessor for EffectHostContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get_world_object(id)
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.world_object_ids()
    }

    fn script_function_known(&self, name: &str) -> bool {
        self.world.script_function_known(name)
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        if self.pending_objects.contains_key(&object.id) {
            host_object_shape_rect(object, self.world.definitions.as_ref())
        } else {
            self.world.object_shape_rect(object)
        }
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        HostWorldContext::definition_metadata(&self.world, id).cloned()
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.object_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.pending_objects.get(&id) else {
                continue;
            };
            if rect.contains_point(object.position.x, object.position.y) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.shape_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.pending_objects.get(&id) else {
                continue;
            };
            if self.object_shape_rect(object).overlaps(&rect) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }
}

fn truncate_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn invert_rgba_alpha(color: u32) -> u32 {
    let alpha = (color >> 24) & 0xff;
    let rgb = color & 0x00ff_ffff;
    ((255 - alpha) << 24) | rgb
}

fn ensure_single_flag(flags: u32, mask: u32, error: &str) -> Result<(), RuntimeError> {
    let masked = flags & mask;
    if masked != 0 && (masked & (masked - 1)) != 0 {
        return Err(RuntimeError::new(error));
    }
    Ok(())
}

pub(crate) fn object_reference_value(id: ObjectId) -> Value {
    Value::Object(id.as_u64())
}

fn object_id_from_value(value: &Value) -> Option<ObjectId> {
    match value {
        Value::Object(id) if *id != 0 => Some(ObjectId::new(*id)),
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id > 0 => Some(ObjectId::new(*id as u64)),
            _ => None,
        },
        _ => None,
    }
}

fn parse_object_reference_argument(
    value: &Value,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    match value {
        Value::Object(_) | Value::Proplist(_) => Ok(object_id_from_value(value)),
        Value::Nil => Ok(None),
        Value::Int(id) if *id == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{}: expected object, proplist, nil, or 0 for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn consume_optional_object_reference_argument(
    args: &[Value],
    index: &mut usize,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    let Some(value) = args.get(*index) else {
        return Ok(None);
    };
    if !matches!(value, Value::Object(_) | Value::Proplist(_) | Value::Nil) {
        return Ok(None);
    }
    let object_id = parse_object_reference_argument(value, function, parameter)?;
    *index += 1;
    Ok(object_id)
}

fn value_to_i32(value: &Value, function: &str, parameter: &str) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(int) => Ok(*int),
        // Unfilled parameter slots are nil and convert to 0; bools convert
        // directly (C4AulExec.cpp:1364-1396 CheckConvertFunctionParameters,
        // C4Value.cpp FnCnvGuess / Bool->Int CnvOK).
        Value::Nil => Ok(0),
        Value::Bool(flag) => Ok(i32::from(*flag)),
        other => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn parse_command_request(
    id: CommandId,
    args: &[Value],
    function: &str,
) -> Result<CommandRequest, RuntimeError> {
    let target = if args.len() > 1 {
        parse_object_reference_argument(&args[1], function, "target")?
    } else {
        None
    };

    let tx = if args.len() > 2 {
        match &args[2] {
            Value::Nil => None,
            other => Some(value_to_i32(other, function, "Tx")?),
        }
    } else {
        None
    };

    let ty = if args.len() > 3 {
        match &args[3] {
            Value::Nil => None,
            other => Some(value_to_i32(other, function, "Ty")?),
        }
    } else {
        None
    };

    let target2 = if args.len() > 4 {
        parse_object_reference_argument(&args[4], function, "target2")?
    } else {
        None
    };

    let update_interval = if args.len() > 5 {
        let interval = value_to_i32(&args[5], function, "update_interval")?;
        if interval < 0 {
            return Err(RuntimeError::new(format!(
                "{}: update interval must be >= 0",
                function
            )));
        }
        interval as u32
    } else {
        0
    };

    let data_value = args.get(6).unwrap_or(&Value::Nil);
    let data = match (id, data_value) {
        (CommandId::Call, Value::String(text)) => CommandData::Text(text.clone()),
        (CommandId::Call, Value::Nil) => CommandData::Text(String::new()),
        (CommandId::Call, other) => {
            return Err(RuntimeError::new(format!(
                "{}: expected string for data when command is Call, got {}",
                function,
                other.type_name()
            )))
        }
        (_, Value::Nil) => CommandData::Integer(0),
        (_, other) => CommandData::Integer(value_to_i32(other, function, "data")?),
    };

    let retries = if args.len() > 7 {
        value_to_i32(&args[7], function, "retries")?
    } else {
        0
    };

    let mode = if args.len() > 8 {
        let raw = value_to_i32(&args[8], function, "mode")?;
        CommandMode::from_i32(raw).unwrap_or(CommandMode::Base)
    } else {
        CommandMode::Base
    };

    Ok(CommandRequest::new(id)
        .with_target(target)
        .with_target2(target2)
        .with_tx(tx)
        .with_ty(ty)
        .with_data(data)
        .with_update_interval(update_interval)
        .with_retries(retries)
        .with_mode(mode))
}

fn parse_player_type_filter(value: Option<&Value>, function: &str) -> Result<i32, RuntimeError> {
    match value {
        Some(Value::Int(filter)) => Ok(*filter),
        Some(Value::Nil) | None => Ok(0),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected int or nil for type filter, got {}",
            function,
            other.type_name()
        ))),
    }
}

fn player_type_matches(_player: &PlayerState, filter: i32) -> bool {
    match filter {
        0 => true,
        1 => true,
        _ => false,
    }
}

fn get_player_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerCount expects at most 1 argument: type",
        ));
    }
    let filter = parse_player_type_filter(args.first(), "GetPlayerCount")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(0));
        };
        let count = context
            .player_ids()
            .iter()
            .filter(|id| {
                context
                    .player_state(**id)
                    .map(|player| player_type_matches(player, filter))
                    .unwrap_or(false)
            })
            .count();
        Ok(Value::Int(truncate_to_i32(count as u64)))
    })
}

fn get_player_by_index(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetPlayerByIndex expects at most 2 arguments: index and type",
        ));
    }
    // C++ pads missing script args with zero: GetPlayerByIndex() is the
    // first player (GoldRush Script1's intro camera).
    let index = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetPlayerByIndex", "index")?,
    };
    let filter = parse_player_type_filter(args.get(1), "GetPlayerByIndex")?;
    if index < 0 {
        return Ok(Value::Int(OWNER_NONE));
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(OWNER_NONE));
        };
        let matching: Vec<i32> = context
            .player_ids()
            .iter()
            .filter_map(|id| {
                context
                    .player_state(*id)
                    .filter(|player| player_type_matches(player, filter))
                    .map(|_| *id)
            })
            .collect();
        let idx = index as usize;
        if idx >= matching.len() {
            Ok(Value::Int(OWNER_NONE))
        } else {
            Ok(Value::Int(matching[idx]))
        }
    })
}

fn get_player_name(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlayerName expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlayerName", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::String(player.name.clone()))
    })
}

fn get_player_id(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlayerID expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlayerID", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if context.player_state(player_id).is_some() {
            Ok(Value::Int(player_id))
        } else {
            Ok(Value::Nil)
        }
    })
}

fn get_player_team(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlayerTeam expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlayerTeam", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        match player.team {
            Some(team) => Ok(Value::Int(team)),
            None => Ok(Value::Nil),
        }
    })
}

fn get_player_type(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlayerType expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlayerType", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if context.player_state(player_id).is_some() {
            Ok(Value::Int(1))
        } else {
            Ok(Value::Nil)
        }
    })
}

fn get_wealth(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetWealth expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetWealth", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.wealth))
    })
}

/// `FnSetWealth` (C4Script.cpp:2761-2766): clamp-set to `0..=100000`,
/// false for invalid players.
fn set_wealth(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "SetWealth expects 2 arguments: player, value",
        ));
    }
    let player_id = value_to_i32(&args[0], "SetWealth", "player")?;
    let value = match args.get(1) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "SetWealth: expected int for value, got {}",
                other.type_name()
            )))
        }
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        let clamped = value.clamp(0, 100_000);
        player.wealth = clamped;
        context.record_player_command(PlayerCommand::SetWealth {
            player_id,
            value: clamped,
        });
        Ok(Value::Bool(true))
    })
}

fn get_score(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetScore expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetScore", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.points))
    })
}

fn get_plr_value(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlrValue expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlrValue", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.value))
    })
}

fn get_plr_value_gain(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlrValueGain expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlrValueGain", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.value_gain))
    })
}

/// FnGetComponent (C4Script.cpp:2685-2709): with `idDef` the def's
/// component list answers; otherwise the object's (scope object when no
/// target). `idComponent` selects the count form, else the indexed form.
/// The object's component ORDER follows its def's list (our object
/// component store is unordered; divergence noted in PORT_STATUS).
fn get_component(args: &[Value]) -> Result<Value, RuntimeError> {
    let component = parse_definition_argument(args.first(), "GetComponent")?;
    let index = parse_optional_i32(args.get(1), "GetComponent", "index")?.unwrap_or(0);
    let target =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "GetComponent", "obj")?;
    let definition = parse_definition_argument(args.get(3), "GetComponent")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let indexed = |components: &[(String, u32)], index: i32| -> Value {
            usize::try_from(index)
                .ok()
                .and_then(|index| components.get(index))
                .map(|(id, _)| Value::C4Id(id.clone()))
                .unwrap_or(Value::Nil)
        };
        if let Some(def_id) = definition {
            let Some(metadata) = context
                .world
                .definition_metadata(&DefinitionId::from(def_id.as_str()))
            else {
                return Ok(Value::Nil);
            };
            if let Some(component) = component {
                let count = metadata
                    .components
                    .iter()
                    .find(|(id, _)| id.eq_ignore_ascii_case(&component))
                    .map(|(_, count)| *count as i32)
                    .unwrap_or(0);
                return Ok(Value::Int(count));
            }
            return Ok(indexed(&metadata.components, index));
        }
        let object = match target {
            Some(id) => context.get_world_object(id),
            None => context
                .object_context()
                .map(|object| object.id())
                .and_then(|id| context.get_world_object(id)),
        };
        let Some(object) = object else {
            return Ok(Value::Nil);
        };
        let state_components = object.full_state().map(|state| state.components.clone());
        let def_order = context
            .world
            .definition_metadata(object.definition_id())
            .map(|metadata| metadata.components.clone())
            .unwrap_or_default();
        if let Some(component) = component {
            let count = state_components
                .as_ref()
                .and_then(|components| {
                    components
                        .iter()
                        .find(|(id, _)| id.as_str().eq_ignore_ascii_case(&component))
                        .map(|(_, count)| *count as i32)
                })
                .or_else(|| {
                    def_order
                        .iter()
                        .find(|(id, _)| id.eq_ignore_ascii_case(&component))
                        .map(|(_, count)| *count as i32)
                })
                .unwrap_or(0);
            return Ok(Value::Int(count));
        }
        Ok(indexed(&def_order, index))
    })
}

/// FnInLiquid (C4Script.cpp:1864-1868): reads the object's CACHED
/// InLiquid flag (updated during movement, C4Movement.cpp:443-460) —
/// never the landscape at call time. Nil without an object.
fn in_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "InLiquid", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if let Some(id) = target {
            if context.object_context().map(|object| object.id()) != Some(id) {
                return Ok(context
                    .get_world_object(id)
                    .map(|object| Value::Bool(object.in_liquid()))
                    .unwrap_or(Value::Nil));
            }
        }
        Ok(context
            .object_context()
            .map(|object| Value::Bool(object.in_liquid()))
            .unwrap_or(Value::Nil))
    })
}

/// FnMaterial (C4Script.cpp:2488-2491): material number by name, -1 when
/// unknown (Game.Material.Get).
fn material(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "Material", "name")?;
    let Some(name) = name else {
        return Ok(Value::Int(MATERIAL_NONE));
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let result = borrow
            .as_ref()
            .and_then(|context| context.world.materials())
            .and_then(|materials| materials.get(&name))
            .map(|material| material.id().index() as i32)
            .unwrap_or(MATERIAL_NONE);
        Ok(Value::Int(result))
    })
}

/// FnGetMaterialVal (C4Script.cpp:4282-4300): a [Material] core entry by
/// compile name; the section must be "Material", the material is an
/// index into the loaded map, out-of-range or unknown entries are nil.
fn get_material_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = parse_optional_string(args.first(), "GetMaterialVal", "entry")?;
    let section = parse_optional_string(args.get(1), "GetMaterialVal", "section")?;
    let material = parse_optional_i32(args.get(2), "GetMaterialVal", "material")?.unwrap_or(0);
    let entry_index = parse_optional_i32(args.get(3), "GetMaterialVal", "entry_nr")?.unwrap_or(0);
    // The material core implies section "Material" (C4Script.cpp:4296).
    if section.as_deref() != Some("Material") {
        return Ok(Value::Nil);
    }
    let (Some(entry), Ok(index)) = (entry, usize::try_from(material)) else {
        return Ok(Value::Nil);
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        Ok(borrow
            .as_ref()
            .and_then(|context| context.world.materials())
            .and_then(|materials| {
                let id = crate::material::MaterialId::new(index)?;
                materials.get_by_id(id)
            })
            .and_then(|material| material.core_entry(&entry, entry_index.max(0) as usize))
            .map(|value| match value.parse::<i32>() {
                Ok(number) => Value::Int(number),
                Err(_) => Value::String(value.to_string()),
            })
            .unwrap_or(Value::Nil))
    })
}

/// FnObjectSetAction (C4Script.cpp:782-789): SetActionByName on ANOTHER
/// object (with start/abort calls). Routed through the reentrancy seam so
/// the target's SetAction host fn runs in the target's scope.
fn object_set_action(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_DEBUG_CHBM").is_ok() {
        tracing::warn!(?args, "OSA ObjectSetAction");
    }
    let Some(target) = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "ObjectSetAction",
        "obj",
    )?
    else {
        return Ok(Value::Bool(false));
    };
    let Some(action) = parse_optional_string(args.get(1), "ObjectSetAction", "action")? else {
        return Ok(Value::Bool(false)); // !szAction
    };
    let mut forwarded: Vec<Value> = vec![Value::String(action)];
    forwarded.extend(args.iter().skip(2).take(3).cloned());
    match call_world_object_function(target, "SetAction", &forwarded) {
        Some(result) => result,
        None => Ok(Value::Bool(false)),
    }
}

/// FnSmoke (C4Script.cpp:2188-2192) -> Smoke (C4Effect.cpp:859-866): with
/// the standard particle system one Smoke particle spawns at
/// (x, y - level/2), size `level`, color `dwClr`.
fn smoke(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut x = value_to_i32(args.first().unwrap_or(&Value::Nil), "Smoke", "x")?;
    let mut y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Smoke", "y")?;
    let level = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Smoke", "level")?;
    let color = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "Smoke", "clr")?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            x = x.saturating_add(position.x);
            y = y.saturating_add(position.y);
        }
        context.register_particle(ParticleCommand::Create(ParticleConfig {
            definition_id: "Smoke".to_string(),
            position: FloatVector2::new(x as f32, (y - level / 2) as f32),
            velocity: FloatVector2::new(0.0, 0.0),
            life: 0,
            parameter_a: level as f32,
            parameter_b: color,
            layer: ParticleLayer::Global,
        }));
        Ok(Value::Nil)
    })
}

/// FnSetPortrait (C4Script.cpp:5333-5341): the visual payload is
/// presentation-only, but the SOURCE DEFINITION is tracked because
/// AdjustPortrait gates a synced `Random(GetPortraitCount())` draw on
/// `GetPortrait(obj, true) != GetID()` (Cowboy.c4d/Script.c:552-564).
fn set_portrait(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "SetPortrait", "portrait")?;
    if name.as_deref().map(str::is_empty).unwrap_or(true) {
        return Ok(Value::Bool(false));
    }
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "SetPortrait", "obj")?;
    let source = match args.get(2) {
        Some(Value::String(id)) | Some(Value::C4Id(id)) if !id.is_empty() => Some(id.clone()),
        _ => None,
    };
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(true));
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Bool(false));
        };
        // idSourceDef 0 falls back to the target's own definition.
        let source = source.or_else(|| {
            context
                .get_world_object(target)
                .map(|object| object.definition_id().to_string())
        });
        let Some(source) = source else {
            return Ok(Value::Bool(false));
        };
        if Some(target) == active {
            if let Some(object) = context.object_context_mut() {
                object.set_portrait_source(source);
            }
        } else if let Some(state) = context.nested_objects.get_mut(&target) {
            state.scope.set_portrait_source(source);
        } else {
            tracing::debug!(
                target = target.as_u64(),
                "SetPortrait: target outside active/nested scopes; source not tracked"
            );
        }
        Ok(Value::Bool(true))
    })
}

/// FnGetPortrait with fGetID (C4Script.cpp:5352-5368): the portrait's
/// source definition id; the object's own definition when never set.
/// The filename variant (fGetID false) is unmodeled - nil.
fn get_portrait(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetPortrait", "obj")?;
    let get_id = args.get(1).map(value_raw_truthy).unwrap_or(false);
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Nil);
        };
        if !get_id {
            tracing::debug!("GetPortrait: filename variant unmodeled; returning nil");
            return Ok(Value::Nil);
        }
        let overridden = if Some(target) == active {
            context
                .object_context()
                .and_then(|object| object.portrait_source_override().cloned())
        } else {
            context
                .nested_objects
                .get(&target)
                .and_then(|state| state.scope.portrait_source_override().cloned())
        };
        // FnGetPortrait (C4Script.cpp:5359-5395): no assigned portrait
        // graphics -> nil. A def-id fallback here broke the cavalry's
        // AdjustPortrait guard (`GetPortrait(this(), true) != GetID()`
        // must be TRUE for a fresh crew NPC so the Random(3) portrait
        // pick runs).
        let source = overridden.or_else(|| {
            context
                .get_world_object(target)
                .and_then(|object| {
                    object
                        .full_state()
                        .map(|state| state.portrait_source.clone())
                })
                .flatten()
        });
        Ok(source.map(Value::C4Id).unwrap_or(Value::Nil))
    })
}

/// FnLandscapeWidth (C4Script.cpp:3077-3080): GBackWdt.
fn landscape_width(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .and_then(|context| context.landscape_ref())
                .map(|landscape| landscape.width() as i32)
                .unwrap_or(0),
        ))
    })
}

/// FnLandscapeHeight (C4Script.cpp:3082-3085): GBackHgt.
fn landscape_height(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .and_then(|context| context.landscape_ref())
                .map(|landscape| landscape.estimated_height())
                .unwrap_or(0),
        ))
    })
}

/// FnFrameCounter (C4Script.cpp): Game.FrameCounter — the current
/// simulation frame.
fn frame_counter(_args: &[Value]) -> Result<Value, RuntimeError> {
    ENVIRONMENT_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let frame = borrow
            .as_ref()
            .map(|context| context.frame as i32)
            .unwrap_or(0);
        Ok(Value::Int(frame))
    })
}

/// The active object of the executing call (`cthr->Obj`).
fn active_object_id() -> Option<ObjectId> {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    })
}

/// C4IdText (C4Id.cpp:26-45) over a script value: C4ID_None -> "NONE",
/// numerical ids 0..9999 -> "%04u", literal ids stay as-is.
fn c4id_text_of(value: &Value) -> String {
    match value {
        Value::C4Id(id) | Value::String(id) if !id.is_empty() => id.clone(),
        Value::Int(raw) => c4id_to_definition(*raw).unwrap_or_else(|| "NONE".to_string()),
        _ => "NONE".to_string(),
    }
}

/// C4ObjectMenu::IsCloseDenied (C4ObjectMenu.cpp:56-75): a USER menu asks
/// MenuQueryCancel(Selection, ParentObject) on the command object
/// (CB_Object) or the scenario script (CB_Scenario); a truthy answer keeps
/// the menu open. The CloseQuerying flag stops recursive queries.
fn menu_close_denied(menu_object: ObjectId, menu: &crate::ObjectMenuState) -> bool {
    thread_local! {
        static CLOSE_QUERYING: RefCell<std::collections::HashSet<ObjectId>> =
            RefCell::new(std::collections::HashSet::new());
    }
    if !menu.user_menu {
        return false;
    }
    let already_querying =
        CLOSE_QUERYING.with(|cell| !cell.borrow_mut().insert(menu_object));
    if already_querying {
        return false;
    }
    let pars = [
        Value::Int(menu.selection),
        object_reference_value(menu_object),
    ];
    // A missing function is a silent miss (the "~" in PSF_MenuQueryCancel);
    // callee errors fall back to close-OK (C4Object::Call, fPassErrors
    // defaults false — the error logs and the call yields C4VNull).
    let denied = match menu.command_object {
        Some(command_object) => {
            call_world_object_own_function(command_object, "MenuQueryCancel", &pars)
        }
        None => HOST_CONTEXT
            .with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.scenario_script().cloned())
            })
            .and_then(|script| call_scoped_script_function(script, "MenuQueryCancel", &pars)),
    }
    .map(|result| result.map(|value| value.as_bool()).unwrap_or(false))
    .unwrap_or(false);
    CLOSE_QUERYING.with(|cell| {
        cell.borrow_mut().remove(&menu_object);
    });
    denied
}

/// C4Object::CloseMenu (C4Object.cpp:2009-2017): force skips the
/// MenuQueryCancel query (C4Menu::TryClose, C4Menu.cpp:317-320); a denied
/// soft close keeps the menu and fails.
fn close_object_menu(target: ObjectId, force: bool) -> bool {
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(menu) = menu else {
        return true; // no menu -> close OK
    };
    if !force && menu_close_denied(target, &menu) {
        return false;
    }
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, None))
            .unwrap_or(false)
    })
}

/// FnCreateMenu (C4Script.cpp:1426-1459) → C4ObjectMenu::Init
/// (C4ObjectMenu.cpp:86-91): closes any old menu (soft — MenuQueryCancel
/// may deny), then installs a fresh user menu with Identification =
/// idMenuID ? idMenuID : iSymbol, the given style, and permanence.
fn create_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let symbol = args.first().cloned().unwrap_or(Value::Nil);
    let menu_target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "CreateMenu",
        "menu object",
    )?;
    let explicit_command = parse_object_reference_argument(
        args.get(2).unwrap_or(&Value::Nil),
        "CreateMenu",
        "command object",
    )?;
    let style = parse_optional_i32(args.get(6), "CreateMenu", "style")?.unwrap_or(0);
    let permanent = args.get(7).map(value_raw_truthy).unwrap_or(false);
    let menu_id = args.get(8).cloned().unwrap_or(Value::Nil);

    let active = active_object_id();
    let Some(target) = menu_target.or(active) else {
        return Ok(Value::Bool(false)); // !pMenuObj && !cthr->Obj
    };
    let command_object = explicit_command.or(active);
    // Object menu: validate the command object (C4Script.cpp:1433-1436);
    // no command object is the scenario-script-callback form.
    if let Some(command_object) = command_object {
        let command_active = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|context| context.object_status_active(command_object))
                .unwrap_or(false)
        });
        if !command_active {
            return Ok(Value::Bool(false));
        }
    }
    // Clear any old menu (C4Script.cpp:1447): a MenuQueryCancel denial
    // aborts the new menu.
    if !close_object_menu(target, false) {
        return Ok(Value::Bool(false));
    }
    let identification = if value_raw_truthy(&menu_id) {
        menu_id
    } else {
        symbol
    };
    let menu = crate::ObjectMenuState {
        identification,
        // Style & C4MN_Style_BaseMask (C4Menu::InitMenu, C4Menu.cpp:359).
        style: style & 127,
        permanent,
        selection: -1,
        user_menu: true,
        command_object,
        items: Vec::new(),
    };
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
            .unwrap_or(false)
    });
    Ok(Value::Bool(stored))
}

/// FnGetMenu (C4Script.cpp:1418-1424): the active menu's Identification;
/// C4MN_None (0) without one; C4ID(-1) without an object.
fn get_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetMenu",
        "obj",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Int(-1));
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    Ok(menu
        .map(|menu| menu.identification)
        .unwrap_or(Value::Int(0)))
}

/// FnPunch (C4Script.cpp:328-332) → ObjectComPunch (C4ObjectCom.cpp:
/// 735-767): a zero punch derives from the Fight physicals
/// (BoundBy(5*attacker/target, 0, 10)); QueryCatchBlow on the target
/// halves punch > 1 and stops the blow; the target loses punch% energy
/// (DoEnergy with the ATTACKER's controller as caused-by, kill-trace
/// marked) and its ComDir stops either way; a stopped blow returns false
/// without a fling; punch >= 10 tries the Tumble action (xdir
/// FIXED100(150)*tdir, ydir -2), the regular path GetPunched (xdir
/// FIXED100(250)*tdir, ydir 0) — each re-writing
/// LastEnergyLossCausePlayer unguarded and firing
/// CatchBlow(punch, attacker) on success.
fn punch(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Punch", "target")?
    else {
        return Ok(Value::Bool(false)); // !pTarget (C4ObjectCom.cpp:737)
    };
    let mut punch = parse_optional_i32(args.get(1), "Punch", "punch")?.unwrap_or(0);

    // Read phase: attacker (cthr->Obj) dir + both Fight physicals.
    let read = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let scope = context.object_context()?;
        let attacker = scope.id();
        let attacker_fight = scope.resolved_physical(false).fight;
        // cObj->Controller — the puncher's kill credit
        // (C4ObjectCom.cpp:749,755,762).
        let attacker_controller = scope.controller();
        // tdir = +1, DIR_Left -> -1 (C4ObjectCom.cpp:745).
        let tdir = match scope.direction() {
            Direction::Left => -1,
            Direction::Right => 1,
        };
        if !context.ensure_object_scope(target) {
            return None;
        }
        let target_fight = context
            .object_scope(target)
            .map(|scope| scope.resolved_physical(false).fight)
            .unwrap_or(0);
        Some((attacker, attacker_fight, attacker_controller, tdir, target_fight))
    });
    let Some((attacker, attacker_fight, attacker_controller, tdir, target_fight)) = read else {
        return Ok(Value::Bool(false)); // !cthr->Obj / unknown target
    };

    if punch == 0 && target_fight != 0 {
        punch = (5 * attacker_fight / target_fight).clamp(0, 10);
    }
    if punch == 0 {
        return Ok(Value::Bool(true)); // nothing to do (C4ObjectCom.cpp:741)
    }

    // PSF_QueryCatchBlow (fail-safe; callee errors log and read as false,
    // C4Object::Call fPassErrors=false).
    let blow_stopped =
        match call_world_object_own_function(target, "QueryCatchBlow", &[object_reference_value(attacker)]) {
            Some(Ok(value)) => value.as_bool(),
            Some(Err(error)) => {
                tracing::warn!(
                    %error,
                    "script error in QueryCatchBlow; continuing like the C++ fail-safe exec"
                );
                false
            }
            None => false,
        };
    if blow_stopped && punch > 1 {
        punch /= 2; // caught blow halves damage (C4ObjectCom.cpp:743)
    }

    // DoEnergy(-punch, false, C4FxCall_EngGetPunched, cObj->Controller)
    // + ComDir stop happen even for stopped blows (C4ObjectCom.cpp:749-752).
    let written = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return false;
        };
        stage_energy_loss_cause(
            context,
            target,
            -punch,
            crate::C4FX_CALL_ENG_GET_PUNCHED,
            attacker_controller,
        );
        context
            .object_scope_mut(target)
            .map(|scope| {
                scope.adjust_energy(-punch, false);
                scope.set_command_direction(CommandDirection::Stop);
            })
            .is_some()
    });
    if !written {
        return Ok(Value::Bool(false));
    }
    if blow_stopped {
        return Ok(Value::Bool(false)); // no tumbles for caught blows
    }

    let try_fling = |action: &str, velocity: FixedVec2| -> bool {
        let action_set = matches!(
            call_world_object_function(target, "SetAction", &[Value::String(action.to_string())]),
            Some(Ok(value)) if value.as_bool()
        );
        if !action_set {
            return false;
        }
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return false;
            };
            context
                .object_scope_mut(target)
                .map(|scope| scope.set_fixed_velocity(velocity))
                .is_some()
        })
    };
    let flung = (punch >= 10
        && try_fling("Tumble", FixedVec2::new(fixed100(150) * tdir, itofix(-2))))
        || try_fling("GetPunched", FixedVec2::new(fixed100(250) * tdir, C4Fixed::ZERO));
    if !flung {
        return Ok(Value::Bool(false));
    }
    // A successful fling writes the kill trace DIRECTLY — no
    // UpdatLastEnergyLossCause guard ("for kill tracing when pushing
    // enemies off a cliff", C4ObjectCom.cpp:755,762).
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            if let Some(scope) = context.object_scope_mut(target) {
                scope.pending_update.energy_loss_cause = Some(attacker_controller);
            }
        }
    });
    // PSF_CatchBlow after a successful fling (C4ObjectCom.cpp:754,762).
    if let Some(Err(error)) = call_world_object_own_function(
        target,
        "CatchBlow",
        &[Value::Int(punch), object_reference_value(attacker)],
    ) {
        tracing::warn!(
            %error,
            "script error in CatchBlow; continuing like the C++ fail-safe exec"
        );
    }
    Ok(Value::Bool(true))
}

/// The literal-text half of the FnAddMenuItem sprintf (C4Script.cpp:
/// 1567-1570, fmt::sprintf(dummy, parameter, 0/1)): specifiers consume the
/// two arguments positionally — the parameter text first (its slot was
/// rewritten "%d" -> "%s"), then the left/right-click flag.
fn sprintf_menu_command(format: &str, parameter: &str, click: i32) -> String {
    let click_text = click.to_string();
    let mut arguments = [parameter, click_text.as_str()].into_iter();
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '%' {
            out.push(current);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some('s') | Some('d') => {
                chars.next();
                out.push_str(arguments.next().unwrap_or(""));
            }
            _ => out.push('%'),
        }
    }
    out
}

/// FnAddMenuItem (C4Script.cpp:1471-1734): appends one item to the menu
/// object's OPEN menu. Sim-observable pieces ported: the composed
/// left/right-click commands (new-style %d sprintf vs old-style
/// "Fn(ID,param[,click][,value])"), the caption's %s -> def-name splice,
/// count/no-count, C4MN_Add_PassValue, selectability, and the
/// first-selectable selection grab (C4Menu::AddItem, C4Menu.cpp:424).
/// Symbols are presentation, but their argument CHECKS still gate the
/// return value (:1626,1679,1690-1693,1705-1709).
fn add_menu_item(args: &[Value]) -> Result<Value, RuntimeError> {
    let caption_arg = parse_optional_string(args.first(), "AddMenuItem", "caption")?;
    let command_arg =
        parse_optional_string(args.get(1), "AddMenuItem", "command")?.unwrap_or_default();
    let item_id = args.get(2).cloned().unwrap_or(Value::Nil);
    let menu_target = parse_object_reference_argument(
        args.get(3).unwrap_or(&Value::Nil),
        "AddMenuItem",
        "menu object",
    )?;
    let mut count = parse_optional_i32(args.get(4), "AddMenuItem", "count")?.unwrap_or(0);
    let parameter = args.get(5).cloned().unwrap_or(Value::Nil);
    // args[6] is the info caption — presentation only (item descriptions
    // are not script-observable).
    let extra = parse_optional_i32(args.get(7), "AddMenuItem", "extra")?.unwrap_or(0);
    let xpar = args.get(8).cloned().unwrap_or(Value::Nil);
    let xpar2 = args.get(9).cloned().unwrap_or(Value::Nil);

    let Some(target) = menu_target.or(active_object_id()) else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1474)
    };
    let (menu, def_name) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return (None, String::new());
        };
        // pDef = C4Id2Def(idItem), falling back to the menu object's own
        // def (C4Script.cpp:1488-1489).
        let item_def = match &item_id {
            Value::C4Id(id) | Value::String(id) if !id.is_empty() => {
                context.definition_metadata(id).map(|meta| meta.name.clone())
            }
            _ => None,
        };
        let def_name = item_def
            .or_else(|| {
                context.get_world_object(target).and_then(|object| {
                    context
                        .definition_metadata(object.definition_id())
                        .map(|meta| meta.name.clone())
                })
            })
            .unwrap_or_default();
        (context.object_menu(target), def_name)
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu (C4Script.cpp:1475)
    };

    // Compose the caption with the def name (C4Script.cpp:1492-1510).
    let mut caption = caption_arg
        .as_deref()
        .map(|text| text.replacen("%s", &def_name, 1))
        .unwrap_or_default();

    // Typed parameter -> command text (C4Script.cpp:1513-1546).
    let parameter_text = match &parameter {
        Value::Int(value) => value.to_string(),
        Value::Bool(flag) => if *flag { "true" } else { "false" }.to_string(),
        Value::C4Id(_) => c4id_text_of(&parameter),
        Value::Object(number) => format!("Object({number})"),
        Value::String(text) => format!("\"{text}\""),
        Value::Nil => "CastAny(0)".to_string(), // C4V_Any raw 0
        Value::Array(_) => {
            return Err(RuntimeError::new("array as parameter to AddMenuItem"));
        }
        Value::Proplist(_) => {
            return Err(RuntimeError::new("map as parameter to AddMenuItem"));
        }
    };

    // C4MN_Add_PassValue payload (C4Script.cpp:1549-1554).
    let own_value = (extra & 128 != 0).then(|| xpar2.as_c4_int().unwrap_or(0));

    // New style (any non-IsIdentifier char, C4Strings.cpp:36-45) vs old
    // style command composition (C4Script.cpp:1556-1597).
    let is_identifier = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '~' | '+' | '-');
    let (command, command2) = if command_arg.chars().any(|c| !is_identifier(c)) {
        let dummy = command_arg.replacen("%d", "%s", 1);
        (
            sprintf_menu_command(&dummy, &parameter_text, 0),
            sprintf_menu_command(&dummy, &parameter_text, 1),
        )
    } else if !command_arg.is_empty() {
        let id_text = c4id_text_of(&item_id);
        match own_value {
            Some(value) => (
                format!("{command_arg}({id_text},{parameter_text},0,{value})"),
                format!("{command_arg}({id_text},{parameter_text},1,{value})"),
            ),
            None => (
                format!("{command_arg}({id_text},{parameter_text})"),
                format!("{command_arg}({id_text},{parameter_text},1)"),
            ),
        }
    } else {
        (String::new(), String::new())
    };

    // Symbol argument checks that gate the return value; the drawing
    // itself is presentation (C4Script.cpp:1600-1723).
    match extra & 127 {
        // C4MN_Add_ImgObjRank / C4MN_Add_ImgObject need an object XPar.
        3 | 4 => {
            if !matches!(xpar, Value::Object(_)) {
                return Ok(Value::Bool(false));
            }
        }
        // C4MN_Add_ImgTextSpec needs a caption (drawn as the symbol,
        // clearing the item caption, C4Script.cpp:1688-1697).
        5 => {
            if caption_arg.is_none() {
                return Ok(Value::Bool(false));
            }
            caption = String::new();
        }
        // C4MN_Add_ImgIndexedColor rejects C4MN_Add_PassValue (:1705-1709).
        7 => {
            if extra & 128 != 0 {
                return Err(RuntimeError::new(
                    "AddMenuItem: C4MN_Add_ImgIndexedColor can not be used together with C4MN_Add_PassValue!",
                ));
            }
        }
        _ => {}
    }

    // Zero count -> no count unless C4MN_Add_ForceCount (C4Script.cpp:1726).
    if count == 0 && extra & 256 == 0 {
        count = 12_345_678; // C4MN_Item_NoCount
    }
    let selectable = !command.is_empty();
    // First selectable item takes the selection, WITHOUT callbacks
    // (C4Menu::AddItem -> SetSelection(ItemCount-1, false, false)).
    if menu.selection == -1 && selectable {
        menu.selection = menu.items.len() as i32;
    }
    menu.items.push(crate::ObjectMenuItem {
        caption,
        command,
        command2,
        count,
        item_id: c4id_text_of(&item_id),
        selectable,
        value: own_value,
    });
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
            .unwrap_or(false)
    });
    Ok(Value::Bool(stored))
}

/// C4ObjectMenu::OnSelectionChanged (C4ObjectMenu.cpp:93-104): user menus
/// fire OnMenuSelection(iNewSelection, ParentObject) on the command object
/// (CB_Object) or the scenario script (CB_Scenario); the result is
/// discarded, a missing function is a silent miss (PSF_MenuSelection is
/// "~"-prefixed), and callee errors log-and-continue (fPassErrors=false).
fn menu_selection_changed(menu_object: ObjectId, menu: &crate::ObjectMenuState) {
    if !menu.user_menu {
        return;
    }
    let pars = [
        Value::Int(menu.selection),
        object_reference_value(menu_object),
    ];
    let result = match menu.command_object {
        Some(command_object) => {
            call_world_object_own_function(command_object, "OnMenuSelection", &pars)
        }
        None => HOST_CONTEXT
            .with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.scenario_script().cloned())
            })
            .and_then(|script| call_scoped_script_function(script, "OnMenuSelection", &pars)),
    };
    if let Some(Err(error)) = result {
        tracing::warn!(
            %error,
            "script error in OnMenuSelection; continuing like the C++ fail-safe exec"
        );
    }
}

/// FnSelectMenuItem (C4Script.cpp:1736-1741) → C4Menu::SetSelection
/// (C4Menu.cpp:557-594): moves the selection only onto SELECTABLE items
/// (or clears it on -1 in an empty menu), returns true whenever a menu is
/// active, and always runs the selection callback with the FINAL selection
/// (fDoCalls=true).
fn select_menu_item(args: &[Value]) -> Result<Value, RuntimeError> {
    let item = parse_optional_i32(args.first(), "SelectMenuItem", "item")?.unwrap_or(0);
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "SelectMenuItem",
        "menu object",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1738)
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu (C4Script.cpp:1739)
    };
    let selectable = usize::try_from(item)
        .ok()
        .and_then(|index| menu.items.get(index))
        .map(|entry| entry.selectable)
        .unwrap_or(false);
    if (item == -1 && menu.items.is_empty()) || selectable {
        menu.selection = item;
    }
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu.clone())))
            .unwrap_or(false)
    });
    if stored {
        menu_selection_changed(target, &menu);
    }
    Ok(Value::Bool(true))
}

/// FnCloseMenu (C4Script.cpp:4309-4314): pObj->CloseMenu(true) — the
/// forced close never asks MenuQueryCancel and always reports success.
fn close_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "CloseMenu",
        "obj",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(close_object_menu(target, true)))
}

/// FnSetPlrView (C4Script.cpp:2545-2550): the player's view target —
/// a viewport/drawing concern (C4PVM_Target); acknowledged no-op like
/// SetPlrViewRange.
fn set_plr_view(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = parse_optional_i32(args.first(), "SetPlrView", "player")?;
    Ok(Value::Bool(true))
}

/// FnSetPlrViewRange (C4Script.cpp:5286-5293): the object's FoW view
/// range — presentation/FoW only (PlrViewRange serializes but the
/// comparator does not cover it); acknowledged (PORT_STATUS).
fn set_plr_view_range(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrViewRange",
        "range",
    )?;
    Ok(Value::Bool(true))
}

/// FnSetSolidMask (C4Script.cpp:271-278): sets the object's SolidMask
/// rect (x,y,wdt,hgt,tx,ty); a zero-area rect disables the mask (gates
/// open/close through this from UpdateTransferZone handlers).
fn set_solid_mask(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut values = [0i32; 6];
    for (i, slot) in values.iter_mut().enumerate() {
        *slot = parse_optional_i32(args.get(i), "SetSolidMask", "rect")?.unwrap_or(0);
    }
    let target =
        parse_object_reference_argument(args.get(6).unwrap_or(&Value::Nil), "SetSolidMask", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Bool(false));
        };
        let rect = crate::DefinitionTargetRect::new(
            values[0], values[1], values[2], values[3], values[4], values[5],
        );
        if Some(target) == active {
            if let Some(object) = context.object_context_mut() {
                object.set_solid_mask_rect(rect);
            }
        } else if let Some(state) = context.nested_objects.get_mut(&target) {
            state.scope.set_solid_mask_rect(rect);
        } else {
            tracing::debug!(
                target = target.as_u64(),
                "SetSolidMask: target outside active/nested scopes; skipped"
            );
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(true))
    })
}

/// FnChangeDef (C4Script.cpp) -> C4Object::ChangeDef
/// (C4Object.cpp:1180-1231): swap the target to a new definition in
/// place. Routed through the pending update; the engine fold performs
/// the swap (action to ActIdle, shape/vertices/category from the new
/// def, solid mask reset).
fn change_def(args: &[Value]) -> Result<Value, RuntimeError> {
    let new_id = match args.first() {
        Some(Value::String(id)) | Some(Value::C4Id(id)) if !id.is_empty() => id.clone(),
        _ => return Ok(Value::Bool(false)),
    };
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "ChangeDef", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(target, "ChangeDef", &[Value::C4Id(new_id)]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let known = context.world.definition_metadata(&new_id).is_some();
        if !known {
            return Ok(Value::Bool(false)); // C4Id2Def miss
        }
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        object.pending_update.change_def = Some(new_id);
        Ok(Value::Bool(true))
    })
}

/// FnGetPlrDownDouble (C4Script.cpp): the player's double-Down control
/// latch. Control input is not replayed into the shadow engine
/// (PORT_STATUS) - always false, like an idle player.
fn get_plr_down_double(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = parse_optional_i32(args.first(), "GetPlrDownDouble", "player")?;
    Ok(Value::Bool(false))
}

/// FnSetVisibility (C4Script.cpp:3860-3869): a draw gate
/// (pObj->Visibility), not modeled in the simulation yet — acknowledged
/// (PORT_STATUS).
fn set_visibility(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetVisibility",
        "visibility",
    )?;
    Ok(Value::Bool(true))
}

/// FnSetClrModulation (C4Script.cpp:3879-3896): graphics color modulation
/// — presentation-only; acknowledged (PORT_STATUS).
fn set_clr_modulation(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetClrModulation",
        "clr",
    )?;
    Ok(Value::Bool(true))
}

/// FnEnter (C4Script.cpp:365-370): pObj (or the scope object) enters the
/// container pTarget (C4Object::Enter; the entry/departure callbacks run
/// when the container change folds, apply_container_change). A FOREIGN
/// subject routes through the reentrancy seam so the change lands in the
/// subject's own scope; the seam re-runs this function with the subject
/// active, which terminates in the self branch.
fn enter(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Enter", "target")?
    else {
        return Ok(Value::Bool(false)); // C4Object::Enter(nullptr)
    };
    let subject =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Enter", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(subject) = subject {
        if Some(subject) != active {
            return match call_world_object_function(
                subject,
                "Enter",
                &[object_reference_value(target)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if object.id() == target {
            return Ok(Value::Bool(false)); // cannot contain itself
        }
        object.set_container(Some(target));
        Ok(Value::Bool(true))
    })
}

/// FnExit (C4Script.cpp:372-390): pObj leaves its container
/// (C4Object::Exit; fails when not contained). The exit position falls to
/// the container-change fold; the optional offset/rotation/speed
/// parameters are not modeled yet (PORT_STATUS).
fn exit_container(args: &[Value]) -> Result<Value, RuntimeError> {
    let subject =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Exit", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(subject) = subject {
        if Some(subject) != active {
            return match call_world_object_function(subject, "Exit", &[]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if object.container().is_none() {
            return Ok(Value::Bool(false)); // not contained
        }
        object.set_container(None);
        Ok(Value::Bool(true))
    })
}

/// FnSetComponent (C4Script.cpp:2659-2663): sets the component count on
/// pObj or the scope object (C4IDList::SetIDCount with fAddNewID — the
/// entry persists even at zero). Foreign subjects route through the seam.
fn set_component(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(component) = parse_definition_argument(args.first(), "SetComponent")? else {
        return Ok(Value::Bool(false));
    };
    let count = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetComponent", "count")?;
    let target =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "SetComponent", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetComponent",
                &args[..2.min(args.len())],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(self_id) = context.object_context().map(|object| object.id()) else {
            return Ok(Value::Bool(false));
        };
        // Read-modify-write: the update replaces the whole map, so seed
        // from pending writes or the object's current components.
        let current = context
            .object_context()
            .and_then(|object| object.pending_update.components.clone())
            .or_else(|| {
                context
                    .get_world_object(self_id)
                    .and_then(|object| object.full_state().map(|state| state.components.clone()))
            })
            .unwrap_or_default();
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        let mut map = current;
        map.insert(DefinitionId::from(component.as_str()), count.max(0) as u32);
        object.pending_update.components = Some(map);
        Ok(Value::Bool(true))
    })
}

/// FnGetDefCoreVal (C4Script.cpp:4170-4180): DefCore reflection. The hot
/// entries real content reads resolve from the definition metadata
/// (Width/Height/Offset from the Shape rect, Value, Mass); anything else
/// is nil with a debug note (PORT_STATUS).
fn get_def_core_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(entry) = parse_optional_string(args.first(), "GetDefCoreVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let _section = parse_optional_string(args.get(1), "GetDefCoreVal", "section")?;
    let requested = parse_definition_argument(args.get(2), "GetDefCoreVal")?;
    let entry_index = parse_optional_i32(args.get(3), "GetDefCoreVal", "entry_nr")?.unwrap_or(0);
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let definition_id = match requested {
            Some(id) => Some(id),
            // `if (!idDef && cthr->Def) idDef = cthr->Def->id` — the
            // executing object's definition.
            None => context
                .object_context()
                .map(|object| object.id())
                .and_then(|id| context.get_world_object(id))
                .map(|object| object.definition_id().to_string()),
        };
        let Some(definition_id) = definition_id else {
            return Ok(Value::Nil);
        };
        let Some(metadata) = context
            .world
            .definition_metadata(&DefinitionId::from(definition_id.as_str()))
        else {
            return Ok(Value::Nil);
        };
        let shape = metadata.shape.unwrap_or(DefinitionRect::new(0, 0, 0, 0));
        Ok(match entry.as_str() {
            "Width" => Value::Int(shape.width),
            "Height" => Value::Int(shape.height),
            "Offset" => Value::Int(if entry_index == 0 { shape.x } else { shape.y }),
            "Value" => Value::Int(metadata.value),
            "Mass" => Value::Int(metadata.mass),
            other => {
                tracing::debug!(entry = other, "GetDefCoreVal entry not modeled; nil");
                Value::Nil
            }
        })
    })
}

fn get_hi_rank(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnGetHiRank (C4Script.cpp:2792-2796) ->
    // C4Player::GetHiRankActiveCrew(false) (C4Player.cpp:1003-1020): walk
    // the crew in order, rank from the linked Info (no info = -1); only a
    // strictly higher rank replaces, so the first of equal ranks wins.
    // CrewDisabled is not tracked yet; the crew list holds active objects.
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetHiRank", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let mut best: Option<(u64, i32)> = None;
        for crew_id in &player.crew {
            let rank = context.world.crew_rank(crew_id.as_u64()).unwrap_or(-1);
            match best {
                Some((_, best_rank)) if best_rank >= rank => {}
                _ => best = Some((crew_id.as_u64(), rank)),
            }
        }
        Ok(best
            .map(|(id, _)| object_reference_value(ObjectId::new(id)))
            .unwrap_or(Value::Nil))
    })
}

fn get_crew(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::new(
            "GetCrew expects exactly 2 arguments: player and index",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetCrew", "player")?;
    let index = value_to_i32(&args[1], "GetCrew", "index")?;
    if index < 0 {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let idx = index as usize;
        let Some(crew_id) = player.crew.get(idx) else {
            return Ok(Value::Nil);
        };
        Ok(object_reference_value(ObjectId::new(crew_id.as_u64())))
    })
}

fn get_crew_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetCrewCount expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetCrewCount", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(truncate_to_i32(player.crew.len() as u64)))
    })
}

fn get_cursor_host(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCursor expects 1 or 2 arguments: player and optional index",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetCursor", "player")?;
    let index = if args.len() == 2 {
        value_to_i32(&args[1], "GetCursor", "index")?
    } else {
        0
    };
    if index < 0 {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        if index == 0 {
            return Ok(player
                .cursor
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        let selection = context.world.crew_selection(player_id);
        let Some(selection) = selection else {
            return Ok(Value::Nil);
        };
        if selection.selected.is_empty() {
            return Ok(Value::Nil);
        }
        let mut remaining = index as usize;
        for crew_id in &player.crew {
            if player.cursor == Some(*crew_id) {
                continue;
            }
            if !selection.selected.contains(crew_id) {
                continue;
            }
            remaining -= 1;
            if remaining == 0 {
                return Ok(object_reference_value(*crew_id));
            }
        }
        Ok(Value::Nil)
    })
}

fn get_view_cursor(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetViewCursor expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetViewCursor", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let focus = player.viewports.first().and_then(|viewport| viewport.focus);
        Ok(focus.map(object_reference_value).unwrap_or(Value::Nil))
    })
}

fn get_select_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetSelectCount expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetSelectCount", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let selection = context
            .world
            .crew_selection(player_id)
            .map(|state| state.selected.len())
            .unwrap_or(0);
        Ok(Value::Int(truncate_to_i32(selection as u64)))
    })
}

fn get_homebase_material(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetHomebaseMaterial expects at least 1 argument: player",
        ));
    }

    let player_id = value_to_i32(&args[0], "GetHomebaseMaterial", "player")?;
    let definition = parse_definition_argument(args.get(1), "GetHomebaseMaterial")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => None,
        Some(value) => Some(value_to_i32(value, "GetHomebaseMaterial", "index")?),
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => None,
        Some(value) => {
            let mask = value_to_i32(value, "GetHomebaseMaterial", "category")?;
            if mask <= 0 {
                None
            } else {
                Some(mask)
            }
        }
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        if let Some(definition) = definition {
            if context.definition_metadata(&definition).is_none()
                && context.definition_category(&definition).is_none()
            {
                return Ok(Value::Nil);
            }
            let count = player
                .home_base_material
                .get(&definition)
                .copied()
                .unwrap_or(0);
            return Ok(Value::Int(count as i32));
        }

        let Some(index) = index else {
            return Ok(Value::Nil);
        };
        if index < 0 {
            return Ok(Value::Nil);
        }

        let entries =
            collect_home_base_entries(player.home_base_material.iter(), category, context);
        let idx = index as usize;
        if idx >= entries.len() {
            return Ok(Value::Nil);
        }
        Ok(Value::String(entries[idx].clone()))
    })
}

fn get_homebase_production(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetHomebaseProduction expects at least 1 argument: player",
        ));
    }

    let player_id = value_to_i32(&args[0], "GetHomebaseProduction", "player")?;
    let definition = parse_definition_argument(args.get(1), "GetHomebaseProduction")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => None,
        Some(value) => Some(value_to_i32(value, "GetHomebaseProduction", "index")?),
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => None,
        Some(value) => {
            let mask = value_to_i32(value, "GetHomebaseProduction", "category")?;
            if mask <= 0 {
                None
            } else {
                Some(mask)
            }
        }
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        if let Some(definition) = definition {
            if context.definition_metadata(&definition).is_none()
                && context.definition_category(&definition).is_none()
            {
                return Ok(Value::Nil);
            }
            let count = player
                .home_base_production
                .get(&definition)
                .copied()
                .unwrap_or(0);
            return Ok(Value::Int(count as i32));
        }

        let Some(index) = index else {
            return Ok(Value::Nil);
        };
        if index < 0 {
            return Ok(Value::Nil);
        }

        let entries =
            collect_home_base_entries(player.home_base_production.iter(), category, context);
        let idx = index as usize;
        if idx >= entries.len() {
            return Ok(Value::Nil);
        }
        Ok(Value::String(entries[idx].clone()))
    })
}

fn get_plr_knowledge(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetPlrKnowledge expects at least 1 argument: player",
        ));
    }

    let player_id = value_to_i32(&args[0], "GetPlrKnowledge", "player")?;
    let definition = parse_definition_argument(args.get(1), "GetPlrKnowledge")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetPlrKnowledge", "index")?,
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => None,
        Some(value) => {
            let mask = value_to_i32(value, "GetPlrKnowledge", "category")?;
            if mask == 0 {
                None
            } else {
                Some(mask)
            }
        }
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        if let Some(definition) = definition {
            let known = player.knowledge.iter().any(|entry| entry == &definition);
            return Ok(Value::Bool(known));
        }

        if index < 0 {
            return Ok(Value::Nil);
        }

        let filtered: Vec<DefinitionId> = player
            .knowledge
            .iter()
            .filter_map(|entry| {
                let metadata = context.definition_metadata(entry)?;
                if let Some(mask) = category {
                    if metadata.category & mask == 0 {
                        return None;
                    }
                }
                Some(entry.clone())
            })
            .collect();

        let idx = index as usize;
        if idx >= filtered.len() {
            return Ok(Value::Nil);
        }

        Ok(Value::String(filtered[idx].clone()))
    })
}

fn set_plr_knowledge(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "SetPlrKnowledge expects 3 arguments: player, definition, remove flag",
        ));
    }

    let player_id = value_to_i32(&args[0], "SetPlrKnowledge", "player")?;
    let definition = match parse_definition_argument(args.get(1), "SetPlrKnowledge")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let remove = match args.get(2) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Nil) | None => false,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "SetPlrKnowledge: expected bool for remove flag, got {}",
                other.type_name()
            )))
        }
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if remove {
            let Some(player) = context.player_state_mut(player_id) else {
                return Ok(Value::Bool(false));
            };
            if let Some(index) = player
                .knowledge
                .iter()
                .position(|entry| entry == &definition)
            {
                let removed = player.knowledge.remove(index);
                context.record_player_command(PlayerCommand::RevokeKnowledge {
                    player_id,
                    definition_id: removed,
                });
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(false))
            }
        } else {
            if context.definition_metadata(&definition).is_none() {
                return Ok(Value::Bool(false));
            }
            let player = match context.player_state_mut(player_id) {
                Some(player) => player,
                None => return Ok(Value::Bool(false)),
            };
            let mut added = false;
            if !player.knowledge.iter().any(|entry| entry == &definition) {
                player.knowledge.push(definition.clone());
                added = true;
            }
            if added {
                context.record_player_command(PlayerCommand::GrantKnowledge {
                    player_id,
                    definition_id: definition.clone(),
                });
            }
            Ok(Value::Bool(true))
        }
    })
}

fn do_homebase_material(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "DoHomebaseMaterial expects 3 arguments: player, definition, change",
        ));
    }

    let player_id = value_to_i32(&args[0], "DoHomebaseMaterial", "player")?;
    let definition = match parse_definition_argument(args.get(1), "DoHomebaseMaterial")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let change = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "DoHomebaseMaterial", "change")?,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if context.definition_metadata(&definition).is_none()
            && context.definition_category(&definition).is_none()
        {
            return Ok(Value::Bool(false));
        }

        let (team_id, updated_material) = {
            let player = match context.player_state_mut(player_id) {
                Some(player) => player,
                None => return Ok(Value::Bool(false)),
            };
            adjust_id_count(
                &mut player.home_base_material,
                &definition,
                change,
                Some(crate::player::MAX_HOME_BASE_MATERIAL),
            );
            (player.team, player.home_base_material.clone())
        };

        if context.team_home_base_rule() {
            if let Some(team) = team_id {
                let teammates: Vec<i32> = context
                    .player_ids()
                    .iter()
                    .copied()
                    .filter(|other_id| {
                        *other_id != player_id
                            && context.player_state(*other_id).and_then(|state| state.team)
                                == Some(team)
                    })
                    .collect();
                for other_id in teammates {
                    if let Some(member) = context.player_state_mut(other_id) {
                        member.home_base_material = updated_material.clone();
                    }
                }
            }
        }

        if change != 0 {
            context.record_player_command(PlayerCommand::AdjustHomeBaseMaterial {
                player_id,
                definition_id: definition,
                delta: change,
            });
        }

        Ok(Value::Bool(true))
    })
}

fn do_homebase_production(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "DoHomebaseProduction expects 3 arguments: player, definition, change",
        ));
    }

    let player_id = value_to_i32(&args[0], "DoHomebaseProduction", "player")?;
    let definition = match parse_definition_argument(args.get(1), "DoHomebaseProduction")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let change = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "DoHomebaseProduction", "change")?,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if context.definition_metadata(&definition).is_none()
            && context.definition_category(&definition).is_none()
        {
            return Ok(Value::Bool(false));
        }

        if context
            .player_state_mut(player_id)
            .map(|player| {
                adjust_id_count(&mut player.home_base_production, &definition, change, None);
            })
            .is_none()
        {
            return Ok(Value::Bool(false));
        }

        if change != 0 {
            context.record_player_command(PlayerCommand::AdjustHomeBaseProduction {
                player_id,
                definition_id: definition,
                delta: change,
            });
        }

        Ok(Value::Bool(true))
    })
}

fn value_to_bool(value: &Value, function: &str, parameter: &str) -> Result<bool, RuntimeError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::Int(int) => Ok(*int != 0),
        Value::Nil => Ok(false),
        other => Err(RuntimeError::new(format!(
            "{}: expected bool or int for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn parse_optional_i32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<i32>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::Int(int)) => Ok(Some(*int)),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn parse_optional_u32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<u32>, RuntimeError> {
    Ok(parse_optional_i32(value, function, parameter)?.map(|raw| raw as u32))
}

fn parse_optional_string(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<String>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        // Falsy parameters reset to nil before the typecheck
        // (C4AulExec.cpp:1364-1396): a literal 0/false in a string slot is
        // a null string, not a conversion error (GoldRush passes 0 for the
        // FindObjectOwner action).
        Some(Value::Int(0)) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected string for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn c4id_to_definition(id: i32) -> Option<String> {
    if id == 0 {
        return None;
    }
    if (0..=9999).contains(&id) {
        return Some(format!("{id:04}"));
    }
    let raw = id as u32;
    let mut bytes = [0u8; 4];
    bytes[0] = (raw & 0x0000_00FF) as u8;
    bytes[1] = ((raw & 0x0000_FF00) >> 8) as u8;
    bytes[2] = ((raw & 0x00FF_0000) >> 16) as u8;
    bytes[3] = ((raw & 0xFF00_0000) >> 24) as u8;
    let end = bytes
        .iter()
        .rposition(|&b| b != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    match String::from_utf8(bytes[..end].to_vec()) {
        Ok(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn parse_definition_argument(
    value: Option<&Value>,
    function: &str,
) -> Result<Option<String>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        // Definition constants are C4ID-typed (C4V_C4ID): FnFindObject and
        // friends declare C4ID parameters, so `FindObject(NOPC)` arrives
        // here as a C4Id value.
        Some(Value::C4Id(id)) => Ok(Some(id.clone())),
        Some(Value::Int(id)) => Ok(c4id_to_definition(*id)),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected definition identifier, got {}",
            function,
            other.type_name()
        ))),
    }
}

fn collect_home_base_entries<'a>(
    entries: impl Iterator<Item = (&'a DefinitionId, &'a u32)>,
    category: Option<i32>,
    context: &EffectHostContext,
) -> Vec<DefinitionId> {
    let mut filtered: Vec<DefinitionId> = entries
        .filter_map(|(definition_id, &count)| {
            if count == 0 {
                return None;
            }
            if let Some(mask) = category {
                let metadata = context.definition_metadata(definition_id.as_str());
                if metadata
                    .map(|meta| meta.category & mask != 0)
                    .unwrap_or(false)
                {
                    Some(definition_id.clone())
                } else {
                    None
                }
            } else {
                Some(definition_id.clone())
            }
        })
        .collect();
    filtered.sort();
    filtered
}

fn adjust_id_count(
    map: &mut HashMap<DefinitionId, u32>,
    definition_id: &DefinitionId,
    delta: i32,
    max: Option<u32>,
) -> u32 {
    match map.entry(definition_id.clone()) {
        Entry::Occupied(mut occupied) => {
            if delta >= 0 {
                let mut new_value = occupied.get().saturating_add(delta as u32);
                if let Some(limit) = max {
                    new_value = new_value.min(limit);
                }
                if new_value == 0 {
                    occupied.remove();
                    0
                } else {
                    occupied.insert(new_value);
                    new_value
                }
            } else {
                let current = *occupied.get();
                let decrease = delta.saturating_abs() as u32;
                if current <= decrease {
                    occupied.remove();
                    0
                } else {
                    let new_value = current - decrease;
                    occupied.insert(new_value);
                    new_value
                }
            }
        }
        Entry::Vacant(vacant) => {
            if delta <= 0 {
                0
            } else {
                let mut new_value = delta as u32;
                if let Some(limit) = max {
                    new_value = new_value.min(limit);
                }
                if new_value == 0 {
                    0
                } else {
                    vacant.insert(new_value);
                    new_value
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerFilter {
    Any,
    Exact(ObjectId),
    RequiresContainer,
    RequiresNoContainer,
}

fn parse_container_filter(
    value: Option<&Value>,
    function: &str,
) -> Result<ContainerFilter, RuntimeError> {
    match value {
        None => Ok(ContainerFilter::Any),
        Some(Value::Nil) => Ok(ContainerFilter::Any),
        Some(Value::Int(raw)) if *raw == ANY_CONTAINER_SENTINEL => {
            Ok(ContainerFilter::RequiresContainer)
        }
        Some(Value::Int(raw)) if *raw == NO_CONTAINER_SENTINEL => {
            Ok(ContainerFilter::RequiresNoContainer)
        }
        Some(Value::Int(raw)) if *raw == 0 => Ok(ContainerFilter::Any),
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            match object_id_from_value(value) {
                Some(id) => Ok(ContainerFilter::Exact(id)),
                None => Err(RuntimeError::new(format!(
                    "{}: expected nonzero object reference for container",
                    function
                ))),
            }
        }
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected object reference or container sentinel, got {}",
            function,
            other.type_name()
        ))),
    }
}

/// `vContainer` (C4Script.cpp:2122-2127): an object filters by that exact
/// container; the NO_CONTAINER/ANY_CONTAINER int sentinels
/// (C4Object.h:83-84) filter by containment; anything else is
/// `C4Value::getObj()` = nil, i.e. no filter — never an error.
fn container_filter_from_value(value: Option<&Value>) -> ContainerFilter {
    match value {
        Some(Value::Int(raw)) if *raw == ANY_CONTAINER_SENTINEL => {
            ContainerFilter::RequiresContainer
        }
        Some(Value::Int(raw)) if *raw == NO_CONTAINER_SENTINEL => {
            ContainerFilter::RequiresNoContainer
        }
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => object_id_from_value(value)
            .map(ContainerFilter::Exact)
            .unwrap_or(ContainerFilter::Any),
        _ => ContainerFilter::Any,
    }
}

fn squared_distance(position: Vector2, x: i32, y: i32) -> i64 {
    let dx = position.x as i64 - x as i64;
    let dy = position.y as i64 - y as i64;
    dx * dx + dy * dy
}

struct FindObjectParams {
    definition: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    ocf_mask: u32,
    action: Option<String>,
    treat_idle: bool,
    action_target: Option<ObjectId>,
    exclude: Option<ObjectId>,
    container: ContainerFilter,
    owner: i32,
    find_next: Option<ObjectId>,
}

impl FindObjectParams {
    /// FnFindObject's script layout (C4Script.cpp:2113-2135): (id, x, y,
    /// wdt, hgt, dwOCF, szAction, pActionTarget, vContainer, pFindNext).
    /// Local calls exclude the caller and adjust x/y by its position
    /// (cthr->Obj). The owner filter is not script-settable here
    /// (C++ passes ANY_OWNER); FindObjectOwner injects it after parsing.
    fn parse_cpp_call(
        args: &[Value],
        function: &str,
        caller: Option<(ObjectId, Vector2)>,
    ) -> Result<Self, RuntimeError> {
        let definition = parse_definition_argument(args.first(), function)?;
        let mut x = parse_optional_i32(args.get(1), function, "x")?.unwrap_or(0);
        let mut y = parse_optional_i32(args.get(2), function, "y")?.unwrap_or(0);
        let width = parse_optional_i32(args.get(3), function, "width")?.unwrap_or(0);
        let height = parse_optional_i32(args.get(4), function, "height")?.unwrap_or(0);
        // Adjust default ocf: an explicit 0 means OCF_All (C4Script.cpp:2120).
        let ocf_mask = parse_optional_u32(args.get(5), function, "ocf")?
            .filter(|&mask| mask != 0)
            .unwrap_or(crate::ocf::ALL);
        let action = parse_optional_string(args.get(6), function, "action")?;
        let treat_idle = matches!(action.as_deref(), Some("Idle") | Some("ActIdle"));
        let action_target = parse_object_reference_argument(
            args.get(7).unwrap_or(&Value::Nil),
            function,
            "action_target",
        )?;
        let container = container_filter_from_value(args.get(8));
        let find_next = parse_object_reference_argument(
            args.get(9).unwrap_or(&Value::Nil),
            function,
            "find_next",
        )?;
        // Local call adjust coordinates (C4Script.cpp:2115-2119).
        if let Some((_, position)) = caller {
            if x != 0 || y != 0 || width != 0 || height != 0 {
                x += position.x;
                y += position.y;
            }
        }
        Ok(Self {
            definition,
            x,
            y,
            width,
            height,
            ocf_mask,
            action,
            treat_idle,
            action_target,
            exclude: caller.map(|(id, _)| id),
            container,
            owner: OWNER_ANY,
            find_next,
        })
    }

    fn parse(args: &[Value]) -> Result<Self, RuntimeError> {
        if args.len() > 12 {
            return Err(RuntimeError::new(
                "FindObject: expected at most 12 arguments",
            ));
        }

        let definition = parse_definition_argument(args.first(), "FindObject")?;
        let x = parse_optional_i32(args.get(1), "FindObject", "x")?.unwrap_or(0);
        let y = parse_optional_i32(args.get(2), "FindObject", "y")?.unwrap_or(0);
        let width = parse_optional_i32(args.get(3), "FindObject", "width")?.unwrap_or(0);
        let height = parse_optional_i32(args.get(4), "FindObject", "height")?.unwrap_or(0);
        let ocf_mask = parse_optional_u32(args.get(5), "FindObject", "ocf")?.unwrap_or(u32::MAX);
        let action = parse_optional_string(args.get(6), "FindObject", "action")?;
        let treat_idle = matches!(action.as_deref(), Some("Idle") | Some("ActIdle"));
        let action_target = parse_object_reference_argument(
            args.get(7).unwrap_or(&Value::Nil),
            "FindObject",
            "action_target",
        )?;
        let exclude = parse_object_reference_argument(
            args.get(8).unwrap_or(&Value::Nil),
            "FindObject",
            "exclude",
        )?;
        let container = parse_container_filter(args.get(9), "FindObject")?;
        let owner = parse_optional_i32(args.get(10), "FindObject", "owner")?.unwrap_or(OWNER_ANY);
        let find_next = parse_object_reference_argument(
            args.get(11).unwrap_or(&Value::Nil),
            "FindObject",
            "find_next",
        )?;

        Ok(Self {
            definition,
            x,
            y,
            width,
            height,
            ocf_mask,
            action,
            treat_idle,
            action_target,
            exclude,
            container,
            owner,
            find_next,
        })
    }

    fn is_full_range(&self) -> bool {
        self.x == 0 && self.y == 0 && self.width == 0 && self.height == 0
    }

    fn is_closest_query(&self) -> bool {
        self.width == -1 && self.height == -1
    }

    fn is_point_query(&self) -> bool {
        !self.is_full_range() && self.width == 0 && self.height == 0
    }

    fn is_rect_query(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    fn matches_object(&self, object: &HostWorldObject) -> bool {
        if matches!(object.status(), ObjectStatus::Deleted) {
            return false;
        }

        if let Some(exclude) = self.exclude {
            if object.id == exclude {
                return false;
            }
        }

        if let Some(definition) = &self.definition {
            if object.definition_id() != definition {
                return false;
            }
        }

        if self.ocf_mask != ocf::ALL && object.ocf() & self.ocf_mask == 0 {
            return false;
        }

        match self.container {
            ContainerFilter::Any => {}
            ContainerFilter::Exact(expected) => {
                if object.container() != Some(expected) {
                    return false;
                }
            }
            ContainerFilter::RequiresContainer => {
                if object.container().is_none() {
                    return false;
                }
            }
            ContainerFilter::RequiresNoContainer => {
                if object.container().is_some() {
                    return false;
                }
            }
        }

        if self.owner != OWNER_ANY && object.owner() != self.owner {
            return false;
        }

        if let Some(target) = self.action_target {
            let matches =
                object.action_target(0) == Some(target) || object.action_target(1) == Some(target);
            if !matches {
                return false;
            }
        }

        if let Some(action) = self.action.as_deref() {
            if !action.is_empty() {
                if self.treat_idle {
                    let name = object.action_name();
                    if name != "Idle" && name != "ActIdle" {
                        return false;
                    }
                } else if object.action_name() != action {
                    return false;
                }
            }
        }

        true
    }

    fn matches_area(&self, world: &impl WorldAccessor, object: &HostWorldObject) -> bool {
        if self.is_full_range() || self.is_closest_query() {
            return true;
        }

        if self.is_point_query() {
            return world
                .object_shape_rect(object)
                .contains_point(self.x, self.y);
        }

        if self.is_rect_query() {
            let position = object.position();
            let dx = position.x - self.x;
            let dy = position.y - self.y;
            return dx >= 0 && dx < self.width && dy >= 0 && dy < self.height;
        }

        false
    }

    fn reference_distance(&self, world: &impl WorldAccessor) -> Option<i64> {
        let id = self.find_next?;
        let object = world.get_object(id)?;
        Some(squared_distance(object.position(), self.x, self.y))
    }

    fn candidate_ids(&self, world: &impl WorldAccessor) -> Vec<ObjectId> {
        if self.is_closest_query() || self.is_full_range() {
            return world.object_ids();
        }

        if self.is_point_query() {
            let rect = DefinitionRect::new(self.x, self.y, 1, 1);
            return world
                .shape_sector_ids_in_rect(rect)
                .unwrap_or_else(|| world.object_ids());
        }

        if self.is_rect_query() {
            let rect = DefinitionRect::new(self.x, self.y, self.width, self.height);
            return world
                .object_sector_ids_in_rect(rect)
                .unwrap_or_else(|| world.object_ids());
        }

        Vec::new()
    }
}

/// FnGetEnergy: `100 * Energy / C4MaxPhysical` — scripts always read
/// percent of the raw physical scale (C4Script.cpp FnGetEnergy).
fn energy_to_script_value(energy: i32) -> i32 {
    ((energy as i64) * 100 / (LEGACY_MAX_PHYSICAL as i64)) as i32
}

fn construction_to_script_value(construction: i32) -> i32 {
    let clamped = construction.clamp(0, FULL_CON);
    ((clamped as i64) * 100 / (FULL_CON as i64)) as i32
}

fn construction_delta_from_percent(percent: i32) -> i32 {
    ((percent as i64) * (FULL_CON as i64) / 100) as i32
}

const LEGACY_MAX_PHYSICAL: i32 = 100_000;
const CONTACT_DIRECTION_MASK: u32 = CNAT_LEFT | CNAT_RIGHT | CNAT_TOP | CNAT_BOTTOM | CNAT_CENTER;

pub(crate) fn compute_vertex_contact(
    landscape: Option<&Landscape>,
    position: Vector2,
    vertex: &ObjectVertex,
    check_mask: u32,
) -> u32 {
    if vertex.cnat & CNAT_NO_COLLISION != 0 {
        return 0;
    }
    let mask = if check_mask == 0 {
        vertex.cnat
    } else {
        check_mask
    };
    let mask = mask & CONTACT_DIRECTION_MASK;
    if mask == 0 {
        return 0;
    }
    let landscape = match landscape {
        Some(value) => value,
        None => return 0,
    };
    let world_x = position.x.saturating_add(vertex.x);
    let world_y = position.y.saturating_add(vertex.y);
    let mut contact = 0;
    if (mask & CNAT_CENTER) != 0 && landscape.is_solid_at(world_x, world_y) {
        contact |= CNAT_CENTER;
    }
    if (mask & CNAT_LEFT) != 0 && landscape.is_solid_at(world_x - 1, world_y) {
        contact |= CNAT_LEFT;
    }
    if (mask & CNAT_RIGHT) != 0 && landscape.is_solid_at(world_x + 1, world_y) {
        contact |= CNAT_RIGHT;
    }
    if (mask & CNAT_TOP) != 0 && landscape.is_solid_at(world_x, world_y - 1) {
        contact |= CNAT_TOP;
    }
    if (mask & CNAT_BOTTOM) != 0 && landscape.is_solid_at(world_x, world_y + 1) {
        contact |= CNAT_BOTTOM;
    }
    contact
}

fn resolve_vertices(
    context: &EffectHostContext,
    target: Option<ObjectId>,
) -> Option<(Vector2, &[ObjectVertex])> {
    if let Some(target_id) = target {
        if let Some(object) = context.object_context() {
            if object.id() == target_id {
                return Some((object.effective_position(), object.vertices()));
            }
        }
        context
            .world
            .get(target_id)
            .map(|other| (other.position(), other.vertices()))
    } else {
        context
            .object_context()
            .map(|object| (object.effective_position(), object.vertices()))
    }
}

const DEFAULT_MAX_ENERGY: i32 = 100;
const DEFAULT_VELOCITY_PRECISION: i32 = 10;

fn normalise_precision(value: i32) -> i32 {
    let value = if value == 0 {
        DEFAULT_VELOCITY_PRECISION
    } else {
        value
    };
    let normalised = value.abs();
    if normalised == 0 {
        1
    } else {
        normalised
    }
}

/// Strips the failsafe marker(s) from a call-family function name:
/// `GetSFunc` strips one leading '~' (C4Aul.cpp:314) and its name-only
/// overload strips a second (C4Aul.cpp:350), so `"~~Name"` resolves to
/// `Name`. Failsafe only changes logging — a miss returns C4VNull either
/// way, so the marker carries no other semantics here.
fn strip_failsafe(name: &str) -> &str {
    let once = name.strip_prefix('~').unwrap_or(name);
    once.strip_prefix('~').unwrap_or(once)
}

/// FnCall (C4Script.cpp:3424-3432): `Call(name, p0..p8)` runs `name` on the
/// calling object itself — `C4Object::Call` (C4Object.cpp:2197-2201) → the
/// object's own def script, script functions ONLY (owner-scoped GetSFunc,
/// C4Aul.cpp:295-298,562-576; engine functions are never found). Nil name,
/// no object context, or a removed `this` → C4VNull; callee errors
/// propagate (fPassErrors=true); script pars[1..=9] shift to Par(0..=8).
fn call_self(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil);
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let target = HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().and_then(|context| {
            context
                .object_context()
                .filter(|scope| !scope.destroy && scope.status.is_active())
                .map(ObjectScopeContext::id)
        })
    });
    let Some(target) = target else {
        return Ok(Value::Nil);
    };
    let pars: Vec<Value> = args.iter().skip(1).take(9).cloned().collect();
    call_world_object_script_function(target, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// FnObjectCall/FnProtectedCall/FnPrivateCall (C4Script.cpp:3434-3449,
/// 3502-3534): run a function on a target object's def script — failsafe
/// resolution (silent C4VNull on a miss), script functions only, NO Status
/// check on the target (unlike `C4Object::Call`). The three access levels
/// (AA_PUBLIC/AA_PROTECTED/AA_PRIVATE) only LOG on violation and the call
/// still executes (C4Aul.cpp:332-342), so one implementation serves all
/// three names. Script pars[2..=9] shift to callee Par(0..=7).
fn object_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) = args.first().and_then(object_id_from_value) else {
        return Ok(Value::Nil); // !pObj → C4VNull (C4Script.cpp:3439)
    };
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil); // !szFunction → C4VNull
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let pars: Vec<Value> = args.iter().skip(2).take(8).cloned().collect();
    call_world_object_script_function(target, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// The VM's cross-object LocalN cell supplier (FnLocalN by-reference
/// foreign-local access, C4Script.cpp:4591-4605): hands out live cells
/// managed by the active host context. `None` for non-object targets —
/// the VM then falls back to the executing object like C++'s nullptr
/// conversion.
fn foreign_local_cell_hook(target: &Value, name: &str) -> Option<lc_script::ValueCell> {
    let target = object_id_from_value(target)?;
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.foreign_local_cell(target, name))
    })
}

/// `obj->Method(args)` / `obj->~Method(args)` — the AB_CALL/AB_CALLFS
/// direct object call (C4AulExec.cpp:1216-1305), forwarded by the VM as
/// [target, name, failsafe, args...]. Resolution is FindSameNameFunc on the
/// target (C4Aul.cpp:130-148): its own script functions first, then
/// global/engine functions running with the TARGET's context. A missing
/// function errors unless failsafe (`->~`), which yields nil; falsy targets
/// were already rejected in the VM.
fn arrow_method_dispatch(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_value = args.first().cloned().unwrap_or(Value::Nil);
    let Some(Value::String(name)) = args.get(1) else {
        return Err(RuntimeError::new(
            "Object call: missing function name".to_string(),
        ));
    };
    let failsafe = args.get(2).map(Value::as_bool).unwrap_or(false);
    let pars: Vec<Value> = args
        .iter()
        .skip(3)
        .collect::<Vec<_>>()
        .into_iter()
        .cloned()
        .collect();

    if let Value::C4Id(def_id) = &target_value {
        // Definition call (C4AulExec.cpp:1235-1245): the definition must be
        // known — that error is NOT covered by the failsafe.
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(def_id).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "Definition call: Definition for id {def_id} not found!"
            )));
        };
        // AB_CALL resolves via FindSameNameFunc: the def's own function
        // first, else a GLOBAL script function running in definition
        // scope (C4AulExec.cpp:1259-1261, C4Aul.cpp:130-148) — unlike
        // FnDefinitionCall's owner-scoped lookup.
        return match call_scoped_script_function_or_global(script, name, &pars) {
            Some(result) => result,
            None if failsafe => Ok(Value::Nil),
            None => Err(RuntimeError::new(format!(
                "Definition call: No function \"{name}\" in definition \"{def_id}\"!"
            ))),
        };
    }

    let Some(target) = object_id_from_value(&target_value) else {
        return Err(RuntimeError::new(format!(
            "Object call: Invalid target type {}, expected object or id!",
            target_value.type_name()
        )));
    };
    // `obj->ID::Func(...)` — the namespace operator (AB_CALLNS): resolve
    // Func in def ID's script and run it on the target. C++ resolves at
    // PARSE time and throws hard errors for a missing def or function
    // (C4AulParse.cpp:3171-3181) — the failsafe `~` does not cover them.
    if let Some((namespace, function)) = name.split_once("::") {
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(namespace).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "direct object call: def not found: {namespace}"
            )));
        };
        if !script.has_function(function) {
            return Err(RuntimeError::new(format!(
                "direct object call: function {namespace}::{function} not found"
            )));
        }
        return match call_world_object_function_in_scope(target, script, function, &pars) {
            Some(result) => result,
            None => Ok(Value::Nil),
        };
    }
    match call_world_object_function(target, name, &pars) {
        Some(result) => result,
        None if failsafe => Ok(Value::Nil),
        None => Err(RuntimeError::new(format!(
            "Object call: No function \"{name}\" in object {target}!"
        ))),
    }
}

/// Runs `function` on a script host with NO object context (Obj=nullptr,
/// C4AulExec.cpp:343): the active object scope is parked on the dormant
/// stack while the nested VM runs, so host functions see no `this`. Used by
/// DefinitionCall and GameCall/GameCallEx. Callee locals are per-call empty
/// (C++ throws on object-local access in a definition call,
/// C4AulExec.cpp:418-420; the Rust VM reads them as nil — documented).
fn call_scoped_script_function(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_scoped_script_function_impl(script, function, args, false)
}

/// The AB_CALL definition-call variant: FindSameNameFunc also finds
/// GLOBAL script functions (C4Aul.cpp:130-148) — own functions win.
fn call_scoped_script_function_or_global(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_scoped_script_function_impl(script, function, args, true)
}

fn call_scoped_script_function_impl(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
    include_globals: bool,
) -> Option<Result<Value, RuntimeError>> {
    let resolvable = if include_globals {
        script.has_function_or_global(function)
    } else {
        script.has_function(function)
    };
    if !resolvable {
        return None;
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            let active = context.object.take();
            context.dormant_scopes.push(active);
        }
    });
    let locals = HashMap::new();
    let call = script.call_with_locals_and_this(function, args, &locals, Value::Nil);
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.object = context.dormant_scopes.pop().unwrap_or(None);
        }
    });
    Some(match call {
        Ok((value, _locals)) => Ok(value),
        Err(lc_script::ScriptError::Runtime(err)) => Err(err),
        Err(other) => Err(RuntimeError::new(other.to_string())),
    })
}

/// FnDefinitionCall (C4Script.cpp:3451-3468): runs a function on a
/// definition's script with Obj=nullptr — always failsafe ("~" prefix,
/// :3457-3459): unknown id or missing function → silent C4VNull. Script
/// pars[2..=9] shift to callee Par(0..=7).
fn definition_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::C4Id(def_id)) = args.first() else {
        return Ok(Value::Nil); // !idID → C4VNull (C4Script.cpp:3456)
    };
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil);
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.definition_script(def_id).cloned())
    });
    let Some(script) = script else {
        return Ok(Value::Nil); // C4Id2Def failure → C4VNull (C4Script.cpp:3462)
    };
    let pars: Vec<Value> = args.iter().skip(2).take(8).cloned().collect();
    call_scoped_script_function(script, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// FnGameCall (C4Script.cpp:3470-3484): runs a function on the scenario
/// script host ONLY (owner-scoped lookup — definition globals are not
/// visible), always failsafe, Obj=nullptr. Script pars[1..=9] shift to
/// callee Par(0..=8).
fn game_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil); // !szFunction → C4VNull (C4Script.cpp:3475)
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.scenario_script().cloned())
    });
    let Some(script) = script else {
        return Ok(Value::Nil);
    };
    let pars: Vec<Value> = args.iter().skip(1).take(9).cloned().collect();
    call_scoped_script_function(script, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// FnGameCallEx (C4Script.cpp:3486-3500) → `C4GameScriptHost::GRBroadcast`
/// (C4ScriptHost.cpp:234-248): calls the function on every LIVE object whose
/// Category has a C4D_Goal|C4D_Rule|C4D_Environment bit, in list order, with
/// results DISCARDED (fRejectTest=false) — "call objects first - scenario
/// script might overwrite hostility, etc." — then on the scenario script,
/// whose result is the sole return value. Always failsafe ("~" prefix);
/// callee errors still pass through (fPassErrors=true).
fn game_call_ex(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil); // !szFunction → C4VNull (C4Script.cpp:3491)
    };
    let name = strip_failsafe(name).to_string();
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let pars: Vec<Value> = args.iter().skip(1).take(9).cloned().collect();

    // C4D_Goal | C4D_Environment | C4D_Rule (definition.rs:1608-1622)
    const BROADCAST_MASK: i32 = (1 << 5) | (1 << 6) | (1 << 19);
    let targets: Vec<ObjectId> = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| {
                context
                    .world_object_ids()
                    .into_iter()
                    .filter(|id| {
                        context
                            .get_world_object(*id)
                            .map(|object| {
                                object.status().is_active()
                                    && object.category() & BROADCAST_MASK != 0
                            })
                            .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    for target in targets {
        // The C++ loop re-checks Status against the live list — skip
        // objects an earlier broadcast call removed.
        let destroyed = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|context| context.nested_object_destroyed(target))
                .unwrap_or(false)
        });
        if destroyed {
            continue;
        }
        if let Some(result) = call_world_object_script_function(target, &name, &pars) {
            result?;
        }
    }

    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.scenario_script().cloned())
    });
    match script {
        Some(script) => call_scoped_script_function(script, &name, &pars).unwrap_or(Ok(Value::Nil)),
        None => Ok(Value::Nil),
    }
}

pub fn register_host_functions(script: &mut ScriptEngine) {
    // Every script host knows the engine constant table
    // (RegisterGlobalConstant, C4Script.cpp:6580-6581).
    crate::script_constants::register_script_constants(script);
    script.register_host_function("AddEffect", add_effect);
    script.register_host_function("RemoveEffect", remove_effect);
    script.register_host_function("GetEffect", get_effect);
    script.register_host_function("GetEffectCount", get_effect_count);
    script.register_host_function("WildcardMatch", wildcard_match);
    script.register_host_function("EffectVar", effect_var);
    script.register_host_function("GetPlayerCount", get_player_count);
    script.register_host_function("GetPlayerByIndex", get_player_by_index);
    script.register_host_function("GetPlayerName", get_player_name);
    script.register_host_function("GetPlayerTeam", get_player_team);
    script.register_host_function("GetPlayerType", get_player_type);
    script.register_host_function("GetPlayerID", get_player_id);
    script.register_host_function("GetWealth", get_wealth);
    script.register_host_function("SetWealth", set_wealth);
    script.register_host_function("GetScore", get_score);
    script.register_host_function("GetPlrValue", get_plr_value);
    script.register_host_function("GetPlrValueGain", get_plr_value_gain);
    script.register_host_function("GetPlrKnowledge", get_plr_knowledge);
    script.register_host_function("GetCrew", get_crew);
    script.register_host_function("GetHiRank", get_hi_rank);
    script.register_host_function("SetComponent", set_component);
    script.register_host_function("GetDefCoreVal", get_def_core_val);
    script.register_host_function("Enter", enter);
    script.register_host_function("Exit", exit_container);
    script.register_host_function("GetComponent", get_component);
    script.register_host_function("InLiquid", in_liquid);
    script.register_host_function("Material", material);
    script.register_host_function("GetMaterialVal", get_material_val);
    script.register_host_function("ObjectSetAction", object_set_action);
    script.register_host_function("Smoke", smoke);
    script.register_host_function("SetPortrait", set_portrait);
    script.register_host_function("GetPortrait", get_portrait);
    script.register_host_function("SetVisibility", set_visibility);
    script.register_host_function("SetPlrViewRange", set_plr_view_range);
    script.register_host_function("AddMenuItem", add_menu_item);
    script.register_host_function("CloseMenu", close_menu);
    script.register_host_function("CreateMenu", create_menu);
    script.register_host_function("GetMenu", get_menu);
    script.register_host_function("SelectMenuItem", select_menu_item);
    script.register_host_function("SetPlrView", set_plr_view);
    script.register_host_function("FrameCounter", frame_counter);
    script.register_host_function("LandscapeWidth", landscape_width);
    script.register_host_function("LandscapeHeight", landscape_height);
    script.register_host_function("SetSolidMask", set_solid_mask);
    script.register_host_function("ChangeDef", change_def);
    script.register_host_function("GetPlrDownDouble", get_plr_down_double);
    script.register_host_function("SetClrModulation", set_clr_modulation);
    script.register_host_function("GetCrewCount", get_crew_count);
    script.register_host_function("GetCursor", get_cursor_host);
    script.register_host_function("GetViewCursor", get_view_cursor);
    script.register_host_function("GetSelectCount", get_select_count);
    script.register_host_function("SetPlrKnowledge", set_plr_knowledge);
    script.register_host_function("SetAction", set_action);
    script.register_host_function("SetBridgeActionData", set_bridge_action_data);
    script.register_host_function("SetActionData", set_action_data);
    script.register_host_function("GetActionData", get_action_data);
    script.register_host_function("GetAction", get_action);
    script.register_host_function("GetCommand", get_command);
    script.register_host_function("ShiftContents", shift_contents);
    script.register_host_function("GetActTime", get_act_time);
    script.register_host_function("GetPhase", get_phase);
    script.register_host_function("SetPhase", set_phase);
    script.register_host_function("GetProcedure", get_procedure);
    script.register_host_function("SetActionTargets", set_action_targets);
    script.register_host_function("GetActionTarget", get_action_target);
    script.register_host_function("GetVertexNum", get_vertex_num);
    script.register_host_function("GetVertex", get_vertex);
    script.register_host_function("GetVertexContact", get_vertex_contact);
    script.register_host_function("Stuck", stuck);
    script.register_host_function("Inside", inside);
    script.register_host_function("GetVisibility", get_visibility);
    script.register_host_function("FinishCommand", finish_command);
    script.register_host_function("SetCrewEnabled", set_crew_enabled);
    script.register_host_function("GetCrewEnabled", get_crew_enabled);
    script.register_host_function("GetChar", get_char);
    script.register_host_function("GetColorDw", get_color_dw);
    script.register_host_function("SetCursor", set_cursor_host);
    script.register_host_function("Fling", fling);
    script.register_host_function("Jump", jump);
    script.register_host_function("Punch", punch);
    script.register_host_function("EnergyCheck", energy_check);
    script.register_host_function("GetContact", get_contact);
    script.register_host_function("PathFree", path_free);
    script.register_host_function("GetPath", get_path);
    script.register_host_function("SetTransferZone", set_transfer_zone);
    script.register_host_function("DigFree", dig_free);
    script.register_host_function("DigFreeRect", dig_free_rect);
    script.register_host_function("FreeRect", free_rect);
    script.register_host_function("ScriptGo", script_go);
    script.register_host_function("BlastFree", blast_free);
    script.register_host_function("ShakeFree", shake_free);
    script.register_host_function("GBackSolid", g_back_solid);
    script.register_host_function("GBackSemiSolid", g_back_semi_solid);
    script.register_host_function("GBackLiquid", g_back_liquid);
    script.register_host_function("GBackSky", g_back_sky);
    script.register_host_function("GetMaterial", get_material);
    script.register_host_function("SetDir", set_dir);
    script.register_host_function("GetDir", get_dir);
    script.register_host_function("SetComDir", set_com_dir);
    script.register_host_function("GetComDir", get_com_dir);
    script.register_host_function("SetCommand", set_command);
    script.register_host_function("AddCommand", add_command);
    script.register_host_function("AppendCommand", append_command);
    script.register_host_function("SetR", set_r);
    script.register_host_function("GetR", get_r);
    script.register_host_function("SetXDir", set_x_dir);
    script.register_host_function("GetXDir", get_x_dir);
    script.register_host_function("SetYDir", set_y_dir);
    script.register_host_function("GetYDir", get_y_dir);
    script.register_host_function("SetRDir", set_r_dir);
    script.register_host_function("GetRDir", get_r_dir);
    script.register_host_function("FindObject", find_object);
    script.register_host_function("FindObjectOwner", find_object_owner);
    script.register_host_function("FindObject2", find_object2);
    script.register_host_function("FindObjects", find_objects_dispatch);
    script.register_host_function("ObjectCount2", object_count2);
    script.register_host_function("ObjectCount", object_count);
    script.register_host_function("ObjectDistance", object_distance);
    script.register_host_function("GetX", get_x);
    script.register_host_function("GetY", get_y);
    script.register_host_function("GetID", get_id);
    script.register_host_function("SetPosition", set_position);
    script.register_host_function("CreateObject", create_object);
    script.register_host_function("CreateConstruction", create_construction);
    script.register_host_function("CreateParticle", create_particle);
    script.register_host_function("CastParticles", cast_particles);
    script.register_host_function("CastBackParticles", cast_back_particles);
    script.register_host_function("PushParticles", push_particles);
    script.register_host_function("ClearParticles", clear_particles);
    script.register_host_function("CustomMessage", custom_message);
    script.register_host_function("Message", message);
    script.register_host_function("PlayerMessage", player_message);
    script.register_host_function("AddMessage", add_message);
    script.register_host_function("PlrMessage", plr_message);
    script.register_host_function("Log", log_message);
    script.register_host_function("DebugLog", debug_log_message);
    script.register_host_function("GameOver", game_over);
    script.register_host_function("Call", call_self);
    script.register_host_function("ObjectCall", object_call);
    script.register_host_function("ProtectedCall", object_call);
    script.register_host_function("PrivateCall", object_call);
    script.register_host_function("DefinitionCall", definition_call);
    script.register_host_function("GameCall", game_call);
    script.register_host_function("GameCallEx", game_call_ex);
    script.register_host_function("Format", format_string);
    script.register_host_function("GetType", get_type);
    script.register_host_function("CreateArray", create_array);
    script.register_host_function("GetLength", get_length);
    script.register_host_function("GetIndexOf", get_index_of);
    script.register_host_function("GetKeys", get_keys);
    script.register_host_function("GetValues", get_values);
    script.register_host_function("Contents", contents);
    script.register_host_function("ContentsCount", contents_count);
    script.register_host_function("FindContents", find_contents);
    script.register_host_function("FindOtherContents", find_other_contents);
    script.register_host_function("Contained", contained);
    script.register_host_function("GetCategory", get_category);
    script.register_host_function("SetCategory", set_category);
    script.register_method_dispatch(std::sync::Arc::new(arrow_method_dispatch));
    script.register_local_cell_hook(std::rc::Rc::new(foreign_local_cell_hook));
    script.register_host_function("NoContainer", no_container);
    script.register_host_function("AnyContainer", any_container);
    script.register_host_function("ActIdle", act_idle);
    script.register_host_function("CreateContents", create_contents);
    script.register_host_function("GetActMapVal", get_act_map_val);
    script.register_host_function("GetObjectVal", get_object_val);
    script.register_host_function("SetEntrance", set_entrance);
    script.register_host_function("SetColorDw", set_color_dw);
    script.register_host_function("SetShape", set_shape);
    script.register_host_function("SetVertex", set_vertex);
    script.register_host_function("SetAlive", set_alive);
    script.register_host_function("GetAlive", get_alive);
    script.register_host_function("SetOwner", set_owner);
    script.register_host_function("GetOwner", get_owner);
    script.register_host_function("Distance", distance);
    script.register_host_function("SetViewOffset", set_view_offset);
    script.register_host_function("GetController", get_controller);
    script.register_host_function("SetController", set_controller);
    script.register_host_function("SetObjectStatus", set_object_status);
    script.register_host_function("GetObjectStatus", get_object_status);
    script.register_host_function("GetObjectLayer", get_object_layer);
    script.register_host_function("GetOCF", get_ocf);
    script.register_host_function("InsertMaterial", insert_material);
    script.register_host_function("ExtractMaterialAmount", extract_material_amount);
    script.register_host_function("IncinerateLandscape", incinerate_landscape);
    script.register_host_function("OnFire", on_fire);
    script.register_host_function("SetGraphics", set_graphics);
    script.register_host_function("SetObjDrawTransform", set_obj_draw_transform);
    script.register_host_function("SetObjDrawTransform2", set_obj_draw_transform2);
    script.register_host_function("RemoveObject", remove_object);
    script.register_host_function("GetEnergy", get_energy);
    script.register_host_function("DoEnergy", do_energy);
    script.register_host_function("GetPhysical", get_physical);
    script.register_host_function("SetPhysical", set_physical);
    script.register_host_function("TrainPhysical", train_physical);
    script.register_host_function("ResetPhysical", reset_physical);
    script.register_host_function("GetBreath", get_breath);
    script.register_host_function("GetName", get_name);
    script.register_host_function("GetCon", get_con);
    script.register_host_function("DoCon", do_con);
    script.register_host_function("DoDamage", do_damage);
    script.register_host_function("DoHomebaseMaterial", do_homebase_material);
    script.register_host_function("DoHomebaseProduction", do_homebase_production);
    script.register_host_function("Random", random);
    script.register_host_function("SetGravity", set_gravity);
    script.register_host_function("GetGravity", get_gravity);
    script.register_host_function("GetHomebaseMaterial", get_homebase_material);
    script.register_host_function("GetHomebaseProduction", get_homebase_production);
    script.register_host_function("SetWind", set_wind);
    script.register_host_function("GetWind", get_wind);
    script.register_host_function("Abs", abs_func);
    script.register_host_function("Min", min_func);
    script.register_host_function("Max", max_func);
    script.register_host_function("Sqrt", sqrt_func);
    script.register_host_function("Mod", modulo);
    script.register_host_function("GetMass", get_mass);
    script.register_host_function("SetMass", set_mass);
    script.register_host_function("GrabObjectInfo", grab_object_info);
    script.register_host_function("MakeCrewMember", make_crew_member);
    script.register_host_function("C4Id", c4_id);
    script.register_host_function("Pow", pow_func);
    script.register_host_function("BoundBy", bound_by_func);
    script.register_host_function("Sin", sin_func);
    script.register_host_function("Cos", cos_func);
    script.register_host_function("SetTemperature", set_temperature);
    script.register_host_function("GetTemperature", get_temperature);
    script.register_host_function("SetClimate", set_climate);
    script.register_host_function("GetClimate", get_climate);
    script.register_host_function("Sound", sound);
    script.register_host_function("SoundLevel", sound_level);
}

/// One synced draw through the active random context (host-side engine
/// draws that C++ makes inside script-called engine functions).
fn draw_context_random(range: i32) -> Result<i32, RuntimeError> {
    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("random context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        Ok(rng.random(range))
    })
}

pub(crate) fn enter_random_context(rng: LcgRng) -> RandomContextGuard {
    RANDOM_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested random contexts are not supported"
        );
        let context = Rc::new(RandomContext {
            rng: RefCell::new(rng),
        });
        *cell.borrow_mut() = Some(context.clone());
        RandomContextGuard {
            context: Some(context),
        }
    })
}

const LEGACY_DEFAULT_MESSAGE_COLOR: u32 = 0x00ff_ffff;

fn value_to_data_string(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Int(i) => i.to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(text) => format!("\"{text}\""),
        Value::C4Id(id) => id.clone(),
        Value::Object(id) => format!("<object {id}>"),
        Value::Array(values) => {
            let inner = values
                .iter()
                .map(value_to_data_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Value::Proplist(entries) => {
            if entries.is_empty() {
                "{}".to_string()
            } else {
                let mut items: Vec<_> = entries.iter().collect();
                items.sort_by(|a, b| a.0.cmp(b.0));
                let inner = items
                    .into_iter()
                    .map(|(key, value)| format!("{key} = {}", value_to_data_string(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {inner} }}")
            }
        }
    }
}

fn format_int_value(value: &Value, function: &str) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(i) => Ok(*i),
        Value::Bool(flag) => Ok(if *flag { 1 } else { 0 }),
        Value::Nil => Ok(0),
        other => Err(RuntimeError::new(format!(
            "{function}: expected integer-compatible value for format placeholder, got {}",
            other.type_name()
        ))),
    }
}

fn render_c4id(raw: i32) -> String {
    if raw == 0 {
        return "NONE".to_string();
    }
    if (0..=9999).contains(&raw) {
        return format!("{raw:04}");
    }
    let bytes = (raw as u32).to_le_bytes();
    let mut text = String::new();
    for byte in bytes {
        if byte == 0 {
            break;
        }
        text.push(byte as char);
    }
    if text.is_empty() {
        "NONE".to_string()
    } else {
        text
    }
}

fn format_c4id_string(value: &Value, function: &str) -> Result<String, RuntimeError> {
    match value {
        Value::Int(raw) => Ok(render_c4id(*raw)),
        // A literal id value (FnFormat %i via C4VID).
        Value::C4Id(id) if !id.is_empty() => Ok(id.clone()),
        Value::C4Id(_) => Ok("NONE".to_string()),
        Value::String(text) if !text.is_empty() => Ok(text.clone()),
        Value::String(_) | Value::Nil => Ok("NONE".to_string()),
        other => Err(RuntimeError::new(format!(
            "{function}: expected C4ID-compatible value for format placeholder, got {}",
            other.type_name()
        ))),
    }
}

fn format_decimal(
    value: i32,
    width: Option<usize>,
    precision: Option<usize>,
    zero_pad: bool,
) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = if value < 0 {
        -(i64::from(value))
    } else {
        i64::from(value)
    };
    let mut digits = if precision == Some(0) && magnitude == 0 {
        String::new()
    } else {
        magnitude.abs().to_string()
    };
    if let Some(prec) = precision {
        if digits.len() < prec {
            let pad = "0".repeat(prec - digits.len());
            digits = format!("{pad}{digits}");
        }
    }
    let mut result = if sign.is_empty() {
        digits.clone()
    } else {
        format!("{sign}{digits}")
    };
    if let Some(width) = width {
        if result.len() < width {
            let pad_len = width - result.len();
            if zero_pad && precision.is_none() {
                let pad = "0".repeat(pad_len);
                if sign.is_empty() {
                    result = format!("{pad}{digits}");
                } else {
                    result = format!("-{pad}{digits}");
                }
            } else {
                let pad = " ".repeat(pad_len);
                result = format!("{pad}{result}");
            }
        }
    }
    result
}

fn format_hex(
    value: i32,
    width: Option<usize>,
    precision: Option<usize>,
    zero_pad: bool,
    uppercase: bool,
) -> String {
    let raw = value as u32;
    let mut digits = if precision == Some(0) && raw == 0 {
        String::new()
    } else if uppercase {
        format!("{raw:X}")
    } else {
        format!("{raw:x}")
    };
    if let Some(prec) = precision {
        if digits.len() < prec {
            let pad = "0".repeat(prec - digits.len());
            digits = format!("{pad}{digits}");
        }
    }
    let mut result = digits.clone();
    if let Some(width) = width {
        if result.len() < width {
            let pad_len = width - result.len();
            if zero_pad && precision.is_none() {
                let pad = "0".repeat(pad_len);
                result = format!("{pad}{digits}");
            } else {
                let pad = " ".repeat(pad_len);
                result = format!("{pad}{result}");
            }
        }
    }
    result
}

fn truncate_to_precision(text: &str, precision: Option<usize>) -> String {
    match precision {
        Some(limit) => text.chars().take(limit).collect(),
        None => text.to_string(),
    }
}

fn pad_left(text: &str, width: Option<usize>) -> String {
    match width {
        Some(width) => {
            let len = text.chars().count();
            if len >= width {
                text.to_string()
            } else {
                let pad = " ".repeat(width - len);
                format!("{pad}{text}")
            }
        }
        None => text.to_string(),
    }
}

fn format_script_string(
    function: &str,
    format_str: &str,
    params: &[Value],
) -> Result<String, RuntimeError> {
    let mut output = String::new();
    let mut chars = format_str.chars().peekable();
    let mut arg_index = 0usize;

    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }

        if matches!(chars.peek(), Some('%')) {
            chars.next();
            output.push('%');
            continue;
        }

        let mut zero_pad = false;
        let mut width_value: Option<usize> = None;
        let mut first_width_digit: Option<char> = None;
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                if first_width_digit.is_none() {
                    first_width_digit = Some(c);
                }
                width_value =
                    Some(width_value.unwrap_or(0) * 10 + c.to_digit(10).unwrap() as usize);
                chars.next();
            } else {
                break;
            }
        }
        if matches!(first_width_digit, Some('0')) && width_value.unwrap_or(0) > 0 {
            zero_pad = true;
        }

        let mut precision: Option<usize> = None;
        if matches!(chars.peek(), Some('.')) {
            chars.next();
            let mut digits = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            precision = Some(digits.parse::<usize>().unwrap_or(0));
        }

        let spec = match chars.next() {
            Some(c) => c,
            None => {
                output.push('%');
                break;
            }
        };

        match spec {
            'd' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_decimal(value, width_value, precision, zero_pad));
            }
            'x' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_hex(value, width_value, precision, zero_pad, false));
            }
            'X' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_hex(value, width_value, precision, zero_pad, true));
            }
            'c' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let code = format_int_value(param, function)? as u32;
                let ch = char::from_u32(code).unwrap_or('?');
                output.push_str(&pad_left(&ch.to_string(), width_value));
            }
            'i' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let text = format_c4id_string(param, function)?;
                let truncated = truncate_to_precision(&text, precision);
                output.push_str(&pad_left(&truncated, width_value));
            }
            's' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let raw = match param {
                    Value::String(text) => text.clone(),
                    Value::Nil => "(null)".to_string(),
                    other => {
                        return Err(RuntimeError::new(format!(
                        "{function}: string format placeholder requires string argument, got {}",
                        other.type_name()
                    )))
                    }
                };
                let truncated = truncate_to_precision(&raw, precision);
                output.push_str(&pad_left(&truncated, width_value));
            }
            'v' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let text = if matches!(param, Value::Nil) {
                    "0".to_string()
                } else {
                    value_to_data_string(param)
                };
                output.push_str(&pad_left(&text, width_value));
            }
            '%' => output.push('%'),
            other => {
                output.push('%');
                output.push(other);
            }
        }
    }

    Ok(output)
}

fn extract_speech_segment(raw: &str) -> Option<String> {
    let mut segments = raw.splitn(3, '$');
    segments.next()?;
    segments
        .next()
        .map(|segment| segment.to_string())
        .filter(|segment| !segment.is_empty())
}

fn extract_message_text(formatted: &str) -> String {
    formatted.split('$').next().unwrap_or("").to_string()
}

fn custom_message(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Ok(Value::Bool(false));
    }

    let message = match &args[0] {
        Value::String(text) if !text.is_empty() => text.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "CustomMessage: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let target = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "CustomMessage", "target")?
    } else {
        None
    };

    let owner = match args.get(2) {
        Some(Value::Nil) | None => OWNER_NONE,
        Some(value) => value_to_i32(value, "CustomMessage", "owner")?,
    };

    let offset_x = match args.get(3) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "x")?,
    };

    let offset_y = match args.get(4) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "y")?,
    };

    let raw_color = match args.get(5) {
        Some(Value::Nil) | None => None,
        Some(value) => Some(value_to_i32(value, "CustomMessage", "color")? as u32),
    };

    let decoration = match args.get(6) {
        Some(Value::Nil) | None => None,
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "CustomMessage: expected string or nil for decoration, got {}",
                other.type_name()
            )))
        }
    };

    let portrait = match args.get(7) {
        Some(Value::Nil) | None => None,
        Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "CustomMessage: expected string or nil for portrait, got {}",
                other.type_name()
            )))
        }
    };

    let flags = match args.get(8) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "flags")? as u32,
    };

    ensure_single_flag(
        flags,
        HORIZONTAL_POSITION_FLAGS,
        "CustomMessage: Only one horizontal positioning flag allowed!",
    )?;
    ensure_single_flag(
        flags,
        VERTICAL_POSITION_FLAGS,
        "CustomMessage: Only one vertical positioning flag allowed!",
    )?;
    ensure_single_flag(
        flags,
        ALIGNMENT_FLAGS,
        "CustomMessage: Only one text alignment flag allowed!",
    )?;

    let width = match args.get(9) {
        Some(Value::Nil) | None => None,
        Some(value) => Some(value_to_i32(value, "CustomMessage", "width")?),
    };

    let color = invert_rgba_alpha(raw_color.unwrap_or(0x00ff_ffff));
    let kind = if target.is_some() {
        if owner != OWNER_NONE {
            MessageKind::TargetPlayer
        } else {
            MessageKind::Target
        }
    } else if owner != OWNER_NONE {
        MessageKind::GlobalPlayer
    } else {
        MessageKind::Global
    };

    let player = if owner == OWNER_NONE {
        None
    } else {
        Some(owner)
    };

    let spec = MessageSpec {
        kind,
        text: message,
        target,
        player,
        offset: Vector2::new(offset_x, offset_y),
        color,
        flags,
        width,
        decoration,
        portrait,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CustomMessage requires an active engine context"))?;
        context.register_message(MessageCommand::Add(spec));
        Ok(Value::Bool(true))
    })
}

enum LogLevel {
    Info,
    Debug,
}

fn log_internal(function: &str, args: &[Value], level: LogLevel) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(format!(
            "{function} expects at least 1 argument: message",
        )));
    }

    let format_str = match &args[0] {
        Value::String(text) => text.clone(),
        Value::Nil => String::new(),
        other => {
            return Err(RuntimeError::new(format!(
                "{function}: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let format_args = if args.len() > 1 { &args[1..] } else { &[] };
    let formatted = format_script_string(function, &format_str, format_args)?;

    match level {
        LogLevel::Info => info!(target: "lc-script", "{}", formatted),
        LogLevel::Debug => debug!(target: "lc-script", "{}", formatted),
    }

    Ok(Value::Bool(true))
}

fn log_message(args: &[Value]) -> Result<Value, RuntimeError> {
    log_internal("Log", args, LogLevel::Info)
}

fn debug_log_message(args: &[Value]) -> Result<Value, RuntimeError> {
    log_internal("DebugLog", args, LogLevel::Debug)
}

fn game_over(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GameOver expects at most 1 argument: game over state",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("GameOver requires an active engine context"))?;
        let triggered = context.request_game_over();
        Ok(Value::Bool(triggered))
    })
}

fn get_keys(args: &[Value]) -> Result<Value, RuntimeError> {
    let map = match args.first() {
        Some(Value::Proplist(map)) => map,
        Some(Value::Nil) | None => {
            return Err(RuntimeError::new("GetKeys(): map expected, got 0"));
        }
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetKeys(): map expected, got {}",
                other.type_name()
            )));
        }
    };

    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort();
    let values = keys.into_iter().map(Value::String).collect();
    Ok(Value::Array(values))
}

fn get_values(args: &[Value]) -> Result<Value, RuntimeError> {
    let map = match args.first() {
        Some(Value::Proplist(map)) => map,
        Some(Value::Nil) | None => {
            return Err(RuntimeError::new("GetValues(): map expected, got 0"));
        }
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetValues(): map expected, got {}",
                other.type_name()
            )));
        }
    };

    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let values = entries
        .into_iter()
        .map(|(_, value)| value.clone())
        .collect();
    Ok(Value::Array(values))
}

fn get_type(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("GetType expects 1 argument: value"));
    }

    let type_code = match &args[0] {
        Value::Int(_) => C4V_INT,
        Value::Bool(_) => C4V_BOOL,
        Value::String(_) => C4V_STRING,
        Value::C4Id(_) => C4V_ID,
        Value::Object(_) => C4V_OBJECT,
        Value::Array(_) => C4V_ARRAY,
        Value::Proplist(_) => C4V_MAP,
        Value::Nil => C4V_ANY,
    };

    Ok(Value::Int(type_code))
}

fn create_array(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("CreateArray expects 1 argument: size"));
    }

    let size = value_to_i32(&args[0], "CreateArray", "size")?;
    if !(0..=LEGACY_MAX_ARRAY_SIZE).contains(&size) {
        return Err(RuntimeError::new(format!(
            "CreateArray: invalid array size ({size})"
        )));
    }

    let values = vec![Value::Nil; size as usize];
    Ok(Value::Array(values))
}

fn get_length(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("GetLength expects 1 argument: value"));
    }

    let value = &args[0];
    if !value.as_bool() {
        return Ok(Value::Nil);
    }

    match value {
        Value::String(text) => {
            let len = i32::try_from(text.chars().count())
                .map_err(|_| RuntimeError::new("GetLength: string length exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        Value::Array(values) => {
            let len = i32::try_from(values.len())
                .map_err(|_| RuntimeError::new("GetLength: array length exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        Value::Proplist(entries) => {
            let len = i32::try_from(entries.len())
                .map_err(|_| RuntimeError::new("GetLength: map entry count exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        _ => Err(RuntimeError::new(
            "func \"GetLength\" par 0 cannot be converted to string or array or map",
        )),
    }
}

fn get_index_of(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetIndexOf expects 2 arguments: value and array",
        ));
    }

    let search = &args[0];
    let array = match &args[1] {
        Value::Array(values) => values,
        Value::Nil => return Ok(Value::Int(-1)),
        _ => return Ok(Value::Int(-1)),
    };

    if let Some(index) = array.iter().position(|entry| entry == search) {
        let index = i32::try_from(index)
            .map_err(|_| RuntimeError::new("GetIndexOf: index exceeds i32 range"))?;
        Ok(Value::Int(index))
    } else {
        Ok(Value::Int(-1))
    }
}

fn format_string(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "Format expects at least 1 argument: format",
        ));
    }

    let format_str = match &args[0] {
        Value::String(text) => text.clone(),
        Value::Nil => String::new(),
        other => {
            return Err(RuntimeError::new(format!(
                "Format: expected string for format, got {}",
                other.type_name()
            )))
        }
    };

    let format_args = if args.len() > 1 { &args[1..] } else { &[] };
    let formatted = format_script_string("Format", &format_str, format_args)?;
    Ok(Value::String(formatted))
}

fn message(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "Message expects at least 1 argument: message",
        ));
    }

    let raw_message = match &args[0] {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "Message: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let target_raw = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "Message", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 2 { &args[2..] } else { &[] };
    let formatted = format_script_string("Message", &raw_message, format_args)?;
    let display_text = extract_message_text(&formatted);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("Message requires an active engine context"))?;

        let mut played_speech = false;
        if let Some(sound) = extract_speech_segment(&raw_message) {
            if !sound.is_empty() {
                let speech_target = target_raw
                    .map(ObjectId::new)
                    .or_else(|| context.object_context().map(|object| object.id()));
                context
                    .audio_mut()
                    .play_sound(&sound, speech_target, 100, false, false, None);
                played_speech = true;
            }
        }

        if !played_speech {
            let text = display_text.clone();
            if !text.trim().is_empty() {
                let spec = MessageSpec {
                    kind: if target_raw.is_some() {
                        MessageKind::Target
                    } else {
                        MessageKind::Global
                    },
                    text,
                    target: target_raw.map(ObjectId::new),
                    player: None,
                    offset: Vector2::ZERO,
                    color: invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR),
                    flags: 0,
                    width: None,
                    decoration: None,
                    portrait: None,
                };
                context.register_message(MessageCommand::Add(spec));
            }
        }

        Ok(Value::Bool(true))
    })
}

fn resolve_target_player(context: &EffectHostContext, player_id: i32) -> Option<i32> {
    if player_id >= 0 && context.player_state(player_id).is_some() {
        Some(player_id)
    } else {
        None
    }
}

fn player_message(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "PlayerMessage expects at least 2 arguments: player and message",
        ));
    }

    let player_id = value_to_i32(&args[0], "PlayerMessage", "player")?;
    let raw_message = match &args[1] {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "PlayerMessage: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let target_raw = if let Some(arg) = args.get(2) {
        parse_object_reference_argument(arg, "PlayerMessage", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 3 { &args[3..] } else { &[] };
    let formatted = format_script_string("PlayerMessage", &raw_message, format_args)?;
    let display_text = extract_message_text(&formatted);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("PlayerMessage requires an active engine context"))?;

        let resolved_player = resolve_target_player(context, player_id);

        let mut played_speech = false;
        if let Some(sound) = extract_speech_segment(&raw_message) {
            if !sound.is_empty() {
                let speech_target = target_raw
                    .map(ObjectId::new)
                    .or_else(|| context.object_context().map(|object| object.id()));
                context
                    .audio_mut()
                    .play_sound(&sound, speech_target, 100, false, false, None);
                played_speech = true;
            }
        }

        if !played_speech {
            let text = display_text.clone();
            if !text.trim().is_empty() {
                let kind = match (target_raw.is_some(), resolved_player.is_some()) {
                    (true, true) => MessageKind::TargetPlayer,
                    (true, false) => MessageKind::Target,
                    (false, true) => MessageKind::GlobalPlayer,
                    (false, false) => MessageKind::Global,
                };
                let spec = MessageSpec {
                    kind,
                    text,
                    target: target_raw.map(ObjectId::new),
                    player: resolved_player,
                    offset: Vector2::ZERO,
                    color: invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR),
                    flags: 0,
                    width: None,
                    decoration: None,
                    portrait: None,
                };
                context.register_message(MessageCommand::Add(spec));
            }
        }

        Ok(Value::Bool(true))
    })
}

fn add_message(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "AddMessage expects at least 1 argument: message",
        ));
    }

    let raw_message = match &args[0] {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddMessage: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let target_raw = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "AddMessage", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 2 { &args[2..] } else { &[] };
    let formatted = format_script_string("AddMessage", &raw_message, format_args)?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("AddMessage requires an active engine context"))?;

        let text = formatted.clone();
        if !text.trim().is_empty() {
            let spec = MessageSpec {
                kind: if target_raw.is_some() {
                    MessageKind::Target
                } else {
                    MessageKind::Global
                },
                text,
                target: target_raw.map(ObjectId::new),
                player: None,
                offset: Vector2::ZERO,
                color: invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR),
                flags: FLAG_MULTIPLE,
                width: None,
                decoration: None,
                portrait: None,
            };
            context.register_message(MessageCommand::Add(spec));
        }

        Ok(Value::Bool(true))
    })
}

fn plr_message(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "PlrMessage expects at least 2 arguments: message and player",
        ));
    }

    let raw_message = match &args[0] {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "PlrMessage: expected string for message, got {}",
                other.type_name()
            )))
        }
    };

    let player_id = value_to_i32(&args[1], "PlrMessage", "player")?;
    let format_args = if args.len() > 2 { &args[2..] } else { &[] };
    let formatted = format_script_string("PlrMessage", &raw_message, format_args)?;
    let display_text = extract_message_text(&formatted);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("PlrMessage requires an active engine context"))?;

        let resolved_player = resolve_target_player(context, player_id);

        let mut played_speech = false;
        if let Some(sound) = extract_speech_segment(&raw_message) {
            if !sound.is_empty() {
                let speech_target = context.object_context().map(|object| object.id());
                context
                    .audio_mut()
                    .play_sound(&sound, speech_target, 100, false, false, None);
                played_speech = true;
            }
        }

        if !played_speech {
            let text = display_text.clone();
            if !text.trim().is_empty() {
                let kind = if resolved_player.is_some() {
                    MessageKind::GlobalPlayer
                } else {
                    MessageKind::Global
                };
                let spec = MessageSpec {
                    kind,
                    text,
                    target: None,
                    player: resolved_player,
                    offset: Vector2::ZERO,
                    color: invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR),
                    flags: 0,
                    width: None,
                    decoration: None,
                    portrait: None,
                };
                context.register_message(MessageCommand::Add(spec));
            }
        }

        Ok(Value::Bool(true))
    })
}

#[derive(Clone, Debug)]
pub(crate) struct HostObjectContext<'a> {
    pub id: ObjectId,
    /// The object's definition — GetID's source when the world snapshot
    /// predates the object (its own materialize-time Initialize).
    pub definition_id: Option<String>,
    pub container: Option<ObjectId>,
    pub status: ObjectStatus,
    pub energy: i32,
    pub damage: i32,
    pub alive: bool,
    /// C4Object::InLiquid (the cached flag FnInLiquid reads).
    pub in_liquid: bool,
    /// C4Object::OwnMass (SetMass leftovers).
    pub own_mass: i32,
    pub owner: i32,
    /// C4Object::Controller (C4Object.h:127) — cause tracing.
    pub controller: i32,
    pub category: i32,
    pub ocf: u32,
    pub ocf_base: u32,
    pub crew_member: bool,
    pub position: Vector2,
    pub velocity: Vector2,
    pub rotation: i32,
    pub effects: &'a [EffectState],
    pub action_name: String,
    pub action_ticks: u32,
    pub action_data: i32,
    pub action_phase: i32,
    pub action_library: ActionLibrary,
    pub direction: Direction,
    pub command_direction: CommandDirection,
    pub command_count: usize,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub vertices: &'a [ObjectVertex],
    pub construction: i32,
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    pub draw_transform: Option<DrawTransform>,
    pub base_graphics: Option<ObjectBaseGraphics>,
    pub info_physical: Option<PhysicalInfo>,
    pub temporary_physical: Option<PhysicalInfo>,
    pub physical_changes: Vec<(String, i32)>,
    pub definition_physical: PhysicalInfo,
}

impl<'a> HostObjectContext<'a> {
    #[cfg(test)]
    pub fn new(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: u32,
        action_data: i32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
        construction: i32,
    ) -> Self {
        Self::with_category(
            id,
            container,
            status,
            energy,
            0,
            construction,
            owner,
            position,
            velocity,
            0,
            effects,
            action_name,
            action_ticks,
            action_data,
            0,
            action_library,
            direction,
            command_direction,
            command_count,
            action_target,
            action_target2,
            vertices,
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
    }

    pub fn with_category(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        construction: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: u32,
        action_data: i32,
        action_phase: i32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
        category: i32,
        ocf_base: u32,
        crew_member: bool,
        draw_transform: Option<DrawTransform>,
        base_graphics: Option<ObjectBaseGraphics>,
    ) -> Self {
        Self {
            id,
            definition_id: None,
            container,
            status,
            energy,
            damage,
            construction: construction.clamp(0, FULL_CON),
            alive: true,
            in_liquid: false,
            own_mass: 0,
            owner,
            // The Init default (C4Object.cpp:162); real controllers ride
            // in via with_controller.
            controller: owner,
            category,
            ocf: ocf::NORMAL,
            ocf_base,
            crew_member,
            position,
            velocity,
            rotation,
            effects,
            action_name: action_name.into(),
            action_ticks,
            action_data,
            action_phase,
            action_library,
            direction,
            command_direction,
            command_count,
            action_target,
            action_target2,
            vertices,
            graphics_overlays: Vec::new(),
            draw_transform,
            base_graphics,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            definition_physical: PhysicalInfo::default(),
        }
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub fn with_controller(mut self, controller: i32) -> Self {
        self.controller = controller;
        self
    }

    pub fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = in_liquid;
        self
    }

    pub fn with_own_mass(mut self, own_mass: i32) -> Self {
        self.own_mass = own_mass;
        self
    }

    pub fn with_definition_id(mut self, definition_id: impl Into<String>) -> Self {
        self.definition_id = Some(definition_id.into());
        self
    }

    #[cfg(test)]
    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = crew_member;
        self
    }

    pub fn with_physicals(
        mut self,
        info: Option<PhysicalInfo>,
        temporary: Option<PhysicalInfo>,
        changes: Vec<(String, i32)>,
        definition: PhysicalInfo,
    ) -> Self {
        self.info_physical = info;
        self.temporary_physical = temporary;
        self.physical_changes = changes;
        self.definition_physical = definition;
        self
    }

    pub fn with_graphics_overlays(mut self, overlays: Vec<ObjectGraphicsOverlay>) -> Self {
        self.graphics_overlays = overlays;
        self
    }

    #[allow(dead_code)]
    pub fn with_draw_transform(mut self, transform: Option<DrawTransform>) -> Self {
        self.draw_transform = transform;
        self
    }

    pub fn with_base_graphics(mut self, base: Option<ObjectBaseGraphics>) -> Self {
        self.base_graphics = base;
        self
    }

    pub fn with_ocf(mut self, ocf: u32) -> Self {
        self.ocf = ocf;
        self
    }

    #[allow(dead_code)]
    pub fn ocf(&self) -> u32 {
        self.ocf
    }

    #[allow(dead_code)]
    pub fn ocf_base(&self) -> u32 {
        self.ocf_base
    }

    #[allow(dead_code)]
    pub fn is_crew_member(&self) -> bool {
        self.crew_member
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PhysicsDelta {
    pub gravity: Option<i32>,
}

impl PhysicsDelta {
    pub fn is_empty(&self) -> bool {
        self.gravity.is_none()
    }

    pub fn apply(&self, physics: &mut PhysicsSettings) {
        if let Some(gravity) = self.gravity {
            physics.gravity = gravity.clamp(-300, 300);
        }
    }
}

#[derive(Debug)]
struct PhysicsContext {
    settings: RefCell<PhysicsSettings>,
    pending: RefCell<PhysicsDelta>,
}

impl PhysicsContext {
    fn new(settings: PhysicsSettings) -> Self {
        Self {
            settings: RefCell::new(settings),
            pending: RefCell::new(PhysicsDelta::default()),
        }
    }

    fn set_gravity(&self, gravity: i32) {
        let clamped = gravity.clamp(-300, 300);
        self.settings.borrow_mut().gravity = clamped;
        self.pending.borrow_mut().gravity = Some(clamped);
    }

    fn gravity(&self) -> i32 {
        self.settings.borrow().gravity
    }

    fn into_delta(self) -> PhysicsDelta {
        self.pending.into_inner()
    }
}

pub(crate) struct PhysicsContextGuard {
    context: Option<Rc<PhysicsContext>>,
}

impl PhysicsContextGuard {
    pub fn finish(mut self) -> PhysicsDelta {
        let context = self
            .context
            .take()
            .expect("physics context already consumed");
        PHYSICS_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("physics context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("physics context still referenced")
            .into_delta()
    }
}

impl Drop for PhysicsContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            PHYSICS_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

pub(crate) fn enter_physics_context(settings: PhysicsSettings) -> PhysicsContextGuard {
    PHYSICS_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested physics contexts are not supported",
        );
        let context = Rc::new(PhysicsContext::new(settings));
        *cell.borrow_mut() = Some(context.clone());
        PhysicsContextGuard {
            context: Some(context),
        }
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EnvironmentDelta {
    pub wind: Option<i32>,
    pub temperature: Option<i32>,
    pub climate: Option<i32>,
}

impl EnvironmentDelta {
    pub(crate) fn is_empty(&self) -> bool {
        self.wind.is_none() && self.temperature.is_none() && self.climate.is_none()
    }

    pub fn apply(&self, environment: &mut EnvironmentSettings) {
        if let Some(wind) = self.wind {
            environment.wind = wind.clamp(-100, 100);
        }
        if let Some(temperature) = self.temperature {
            environment.temperature = temperature.clamp(-100, 100);
        }
        if let Some(climate) = self.climate {
            environment.climate = climate.clamp(-50, 50);
        }
    }
}

#[derive(Debug)]
struct EnvironmentContext {
    settings: RefCell<EnvironmentSettings>,
    frame: u64,
    pending: RefCell<EnvironmentDelta>,
}

impl EnvironmentContext {
    fn new(settings: EnvironmentSettings, frame: u64) -> Self {
        Self {
            settings: RefCell::new(settings),
            frame,
            pending: RefCell::new(EnvironmentDelta::default()),
        }
    }

    fn set_wind(&self, wind: i32) {
        let clamped = wind.clamp(-100, 100);
        self.settings.borrow_mut().wind = clamped;
        self.pending.borrow_mut().wind = Some(clamped);
    }

    fn wind_force(&self) -> i32 {
        let settings = self.settings.borrow();
        settings.wind_force(self.frame)
    }

    fn set_temperature(&self, temperature: i32) {
        let clamped = temperature.clamp(-100, 100);
        self.settings.borrow_mut().temperature = clamped;
        self.pending.borrow_mut().temperature = Some(clamped);
    }

    fn ambient_temperature(&self) -> i32 {
        let settings = self.settings.borrow();
        settings.ambient_temperature(self.frame)
    }

    fn set_climate(&self, climate: i32) {
        let clamped = climate.clamp(-50, 50);
        self.settings.borrow_mut().climate = clamped;
        self.pending.borrow_mut().climate = Some(clamped);
    }

    fn climate(&self) -> i32 {
        self.settings.borrow().climate
    }

    fn into_delta(self) -> EnvironmentDelta {
        self.pending.into_inner()
    }
}

pub(crate) struct EnvironmentContextGuard {
    context: Option<Rc<EnvironmentContext>>,
}

impl EnvironmentContextGuard {
    pub fn finish(mut self) -> EnvironmentDelta {
        let context = self
            .context
            .take()
            .expect("environment context already consumed");
        ENVIRONMENT_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("environment context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("environment context still referenced")
            .into_delta()
    }
}

impl Drop for EnvironmentContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            ENVIRONMENT_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

pub(crate) fn enter_environment_context(
    settings: EnvironmentSettings,
    frame: u64,
) -> EnvironmentContextGuard {
    ENVIRONMENT_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested environment contexts are not supported",
        );
        let context = Rc::new(EnvironmentContext::new(settings, frame));
        *cell.borrow_mut() = Some(context.clone());
        EnvironmentContextGuard {
            context: Some(context),
        }
    })
}

#[allow(dead_code)]
pub(crate) fn with_effect_context<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
{
    with_effect_context_with_state(object, global_effects, world, next_object_id, false, func)
}

pub(crate) fn with_effect_context_with_state<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    game_over_triggered: bool,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
{
    let audio_state = AUDIO_CONTEXT
        .with(|cell| cell.borrow_mut().take())
        .unwrap_or_default();
    HOST_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested effect contexts are not supported"
        );
        *cell.borrow_mut() = Some(EffectHostContext::new(
            object,
            global_effects.to_vec(),
            world,
            next_object_id,
            audio_state,
            game_over_triggered,
        ));
        let result = func();
        let context = cell
            .borrow_mut()
            .take()
            .expect("effect context must be present");
        let outcome = context.into_commands();
        AUDIO_CONTEXT.with(|cell| {
            *cell.borrow_mut() = Some(outcome.audio.state.clone());
        });
        (result, outcome)
    })
}

#[derive(Debug, Clone)]
pub(crate) enum LandscapeOperation {
    DigCircle {
        center: Vector2,
        radius: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    },
    DigRect {
        origin: Vector2,
        width: i32,
        height: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    },
    /// FnFreeRect -> Landscape::ClearRect (C4Script.cpp:3125-3131): the
    /// rect clears outright — no dug-out material, no PXS.
    ClearRect {
        origin: Vector2,
        width: i32,
        height: i32,
    },
    BlastCircle {
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    },
    ShakeCircle {
        center: Vector2,
        radius: i32,
    },
    /// FnInsertMaterial -> Landscape::InsertMaterial
    /// (C4Script.cpp:2207-2211): caller-relative coordinates, material by
    /// number.
    InsertMaterial {
        material: i32,
        position: Vector2,
        velocity: Vector2,
    },
    /// FnExtractMaterialAmount (C4Script.cpp:2264-2273): the count was
    /// simulated at call time; the apply reruns the REAL loop.
    ExtractMaterialAmount {
        material: i32,
        position: Vector2,
        amount: i32,
    },
}

/// Side effects a nested script call (Find_Func/GameCall reentrancy) made to
/// an object other than the outer call's `this`. Folded out of the nested
/// scope in first-call order; the engine applies them after the outer
/// object's update.
#[derive(Debug, Clone)]
pub(crate) struct NestedObjectOutcome {
    pub object_id: ObjectId,
    pub effects: Vec<EffectCommand>,
    pub update: Option<ObjectUpdate>,
    pub commands: Vec<QueuedCommand>,
    pub command_operations: Vec<CommandOperation>,
    pub destroy: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectContextOutcome {
    pub object: Vec<EffectCommand>,
    pub global: Vec<EffectCommand>,
    pub object_update: Option<ObjectUpdate>,
    pub object_commands: Vec<QueuedCommand>,
    pub command_operations: Vec<CommandOperation>,
    pub destroy_object: bool,
    /// Mutations nested script calls made to other objects, in first-call
    /// order. C++ mutates live state during the call; the copy-in/copy-out
    /// architecture applies them when the outer call commits.
    pub other_objects: Vec<NestedObjectOutcome>,
    pub environment: Option<EnvironmentDelta>,
    pub physics: Option<PhysicsDelta>,
    pub spawns: Vec<SpawnConfig>,
    pub landscape: Vec<LandscapeOperation>,
    pub particles: Vec<ParticleCommand>,
    pub transfer_zones: Vec<TransferZoneCommand>,
    pub messages: Vec<MessageCommand>,
    pub player_commands: Vec<PlayerCommand>,
    pub audio: AudioOutcome,
    pub trigger_game_over: bool,
    pub script_go: Option<bool>,
    pub next_object_id: u64,
    /// VM-final locals of an effect callback that ran in its command
    /// target's own context (pFn->Exec(pCommandTarget, ...),
    /// C4Effect.cpp:129): the dispatch layer records them, the effect
    /// event loop persists them onto the object.
    pub context_locals: Option<HashMap<String, Value>>,
}

impl EffectContextOutcome {
    fn new(
        object: Vec<EffectCommand>,
        global: Vec<EffectCommand>,
        object_update: Option<ObjectUpdate>,
        object_commands: Vec<QueuedCommand>,
        command_operations: Vec<CommandOperation>,
        destroy_object: bool,
        environment: Option<EnvironmentDelta>,
        physics: Option<PhysicsDelta>,
        spawns: Vec<SpawnConfig>,
        landscape: Vec<LandscapeOperation>,
        transfer_zones: Vec<TransferZoneCommand>,
        messages: Vec<MessageCommand>,
        player_commands: Vec<PlayerCommand>,
        audio: AudioOutcome,
        trigger_game_over: bool,
        script_go: Option<bool>,
        next_object_id: u64,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
            other_objects: Vec::new(),
            environment,
            physics,
            spawns,
            landscape,
            particles: Vec::new(),
            transfer_zones,
            messages,
            player_commands,
            audio,
            trigger_game_over,
            script_go,
            next_object_id,
            context_locals: None,
        }
    }

    pub(crate) fn empty(next_object_id: u64, audio: AudioRegistry) -> Self {
        Self {
            object: Vec::new(),
            global: Vec::new(),
            object_update: None,
            object_commands: Vec::new(),
            command_operations: Vec::new(),
            destroy_object: false,
            other_objects: Vec::new(),
            environment: None,
            physics: None,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
            transfer_zones: Vec::new(),
            messages: Vec::new(),
            player_commands: Vec::new(),
            audio: AudioOutcome {
                state: audio,
                events: Vec::new(),
            },
            trigger_game_over: false,
            script_go: None,
            next_object_id,
            context_locals: None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum EffectScope {
    Object,
    Global,
}

#[derive(Debug)]
struct RandomContext {
    rng: RefCell<LcgRng>,
}

impl RandomContext {
    fn into_rng(self) -> LcgRng {
        self.rng.into_inner()
    }
}

pub(crate) struct RandomContextGuard {
    context: Option<Rc<RandomContext>>,
}

impl RandomContextGuard {
    pub fn finish(mut self) -> LcgRng {
        let context = self
            .context
            .take()
            .expect("random context already consumed");
        RANDOM_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("random context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("random context still referenced")
            .into_rng()
    }
}

impl Drop for RandomContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            RANDOM_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

/// `SWildcardMatchEx` (C4Strings.cpp:531-562): `*`/`?` wildcard match with
/// backtracking, byte-wise like the C++ char loop.
fn s_wildcard_match_ex(string: &str, wildcard: &str) -> bool {
    let (s, w) = (string.as_bytes(), wildcard.as_bytes());
    let (mut pos, mut wild) = (0usize, 0usize);
    let mut backtrack: Option<(usize, usize)> = None;
    while wild < w.len() || backtrack.is_some() {
        if w.get(wild) == Some(&b'*') {
            wild += 1;
            backtrack = Some((wild, pos));
        } else if pos >= s.len() {
            break;
        } else if w.get(wild) == Some(&b'?') || w.get(wild) == Some(&s[pos]) {
            wild += 1;
            pos += 1;
        } else if let Some((last_wild, last_pos)) = backtrack {
            backtrack = Some((last_wild, last_pos + 1));
            wild = last_wild;
            pos = last_pos + 1;
        } else {
            return false;
        }
    }
    wild >= w.len() && pos >= s.len()
}

/// `FnWildcardMatch` (C4Script.cpp:5606-5609): both params go through
/// `FnStringPar`, which maps nil (and Set0'd falsy pars,
/// C4AulExec.cpp:1370-1374) to `""` (C4Script.cpp:78-81).
fn wildcard_match(args: &[Value]) -> Result<Value, RuntimeError> {
    let string_par = |value: Option<&Value>, par: &str| -> Result<String, RuntimeError> {
        match value {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(Value::Nil) | Some(Value::Int(0)) | Some(Value::Bool(false)) | None => {
                Ok(String::new())
            }
            Some(other) => Err(RuntimeError::new(format!(
                "WildcardMatch: expected string or nil for {par}, got {}",
                other.type_name()
            ))),
        }
    };
    let string = string_par(args.first(), "string")?;
    let wildcard = string_par(args.get(1), "wildcard")?;
    Ok(Value::Int(i32::from(s_wildcard_match_ex(
        &string, &wildcard,
    ))))
}

/// Effect-name parameter shared by AddEffect/RemoveEffect/GetEffect/
/// GetEffectCount: C++ declares it `C4String *`, and pre-#strict-3 callers
/// legally pass falsy values that CheckConvertFunctionParameters Set0()s to
/// nil before conversion (C4AulExec.cpp:1370-1374); the empty string also
/// means "match all" (C4Script.cpp:5561). Truthy non-strings throw in C++.
fn effect_name_filter<'a>(
    function: &str,
    value: &'a Value,
) -> Result<Option<&'a str>, RuntimeError> {
    match value {
        Value::String(name) if !name.is_empty() => Ok(Some(name.as_str())),
        Value::String(_) | Value::Nil | Value::Int(0) | Value::Bool(false) => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{function}: expected string or nil for name, got {}",
            other.type_name()
        ))),
    }
}

/// Effect host functions accept ANY object as the state target (the
/// C4Effect operations attach to the GIVEN object, C4Effect.cpp): a
/// foreign target re-dispatches through the reentrancy seam so the
/// effect operation runs in the target's own scope (and folds with its
/// nested outcome). Returns None when the state is not a foreign object
/// — the caller proceeds locally.
fn redirect_foreign_effect_target(
    function: &'static str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let target = match args.get(1) {
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => object_id_from_value(value)?,
        _ => return None,
    };
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if Some(target) == active {
        return None;
    }
    Some(match call_world_object_function(target, function, args) {
        Some(result) => result,
        None => Ok(Value::Int(0)),
    })
}

fn add_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "AddEffect expects at least 2 arguments: name and state",
        ));
    }

    let name = match effect_name_filter("AddEffect", &args[0])? {
        Some(name) => name.to_owned(),
        None => return Ok(Value::Int(0)),
    };

    if let Some(result) = redirect_foreign_effect_target("AddEffect", args) {
        return result;
    }

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Object(_) | Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected object or proplist for object state, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let priority = match args.get(2) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 100,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected int for priority, got {}",
                other.type_name()
            )))
        }
    };

    if priority == 0 {
        return Ok(Value::Int(0));
    }

    // C++ FnAddEffect: unpassed iTimerIntervall is 0 - no timer callbacks
    // (C4Effect.cpp:342).
    let interval = match args.get(3) {
        Some(Value::Int(value)) if *value >= 0 => *value,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "AddEffect: interval must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected int for interval, got {}",
                other.type_name()
            )))
        }
    };

    let len = args.len();
    let mut idx = 4;
    let mut command_target: Option<i32> = None;
    let mut command_target_id: Option<String> = None;
    let mut timer: Option<i32> = None;
    let mut vars: Vec<EffectVarValue> = Vec::new();

    if idx < len {
        match &args[idx] {
            Value::Object(_) | Value::Proplist(_) | Value::Nil => {
                command_target = parse_command_target(&args[idx])?;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && len > idx + 1 => {
                command_target = None;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && len == idx + 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Int(value) if len == idx + 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected proplist, nil, or int for command target, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if idx < len {
        match &args[idx] {
            Value::String(_) | Value::Nil => {
                command_target_id = parse_command_target_id(&args[idx])?;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && idx < len - 1 => {
                command_target_id = None;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && timer.is_none() && idx == len - 1 => {
                command_target_id = None;
                idx += 1;
            }
            Value::Int(value) if timer.is_none() && idx == len - 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Int(_) => {
                return Err(RuntimeError::new(
                    "AddEffect: command target id must be string, nil, or 0",
                ));
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected string or nil for command target id, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if idx < len {
        match &args[idx] {
            Value::Int(value) if timer.is_none() => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Nil => {
                idx += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected int or nil for timer, got {}",
                    other.type_name()
                )));
            }
        }
    }

    while idx < len {
        let value = &args[idx];
        vars.push(value_to_effect_var(value));
        idx += 1;
    }

    // C4Effect's ctor runs Fx*Start synchronously inside FnAddEffect
    // (C4Effect.cpp:96-152). Priority-1 effects skip the check chain
    // entirely (:170), so their Start can run right here through the
    // nested-call seam; other priorities keep the deferred event path
    // (their check chain still runs there first).
    let synchronous_start = priority == 1 && command_target.is_some();
    let effect_name = name.clone();
    let call_vars: Vec<Value> = vars.iter().take(4).map(effect_var_to_value).collect();
    let for_object = match args.get(1) {
        Some(value @ Value::Object(_)) => value.clone(),
        _ => Value::Nil,
    };

    let identifier = with_context_mut(scope, move |ctx| {
        let mut effect = EffectState::new(name)
            .with_priority(priority)
            .with_interval(interval);
        if let Some(timer) = timer {
            effect = effect.with_timer(timer);
        }
        effect = effect.with_command_target(command_target);
        effect = effect.with_command_id(command_target_id);
        if !vars.is_empty() {
            effect = effect.with_vars(vars);
        }
        effect.start_dispatched = synchronous_start;
        ctx.add_effect(effect)
    })?;

    if synchronous_start && identifier > 0 {
        if let Some(target) = command_target {
            let callback = format!("Fx{effect_name}Start");
            let mut call_args = vec![for_object, Value::Int(identifier), Value::Int(0)];
            call_args.extend(call_vars);
            call_args.resize(7, Value::Nil);
            if let Some(result) = call_world_object_script_function(
                ObjectId::new(target as u64),
                &callback,
                &call_args,
            ) {
                // fPassErrors=true (C4Script.cpp:5451): errors abort the
                // calling script like C++.
                let outcome = result?;
                if matches!(outcome, Value::Int(-1)) {
                    // C4Fx_Start_Deny: the effect dies without a Stop
                    // callback (C4Effect.cpp:128-131).
                    with_context_mut(scope, |ctx| {
                        ctx.remove_effect(None, identifier as usize, true);
                        0
                    })?;
                }
            }
        }
    }

    Ok(Value::Int(identifier))
}

fn remove_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("RemoveEffect", args) {
        return result;
    }
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "RemoveEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = effect_name_filter("RemoveEffect", &args[0])?.map(str::to_owned);

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Object(_) | Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected object or proplist for object state, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let index = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => *value as usize,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "RemoveEffect: index must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "RemoveEffect: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let mut no_callbacks = false;
    if let Some(flag) = args.get(3) {
        match flag {
            Value::Bool(value) => no_callbacks = *value,
            Value::Nil => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected bool or nil for no-call flag, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let removed = with_context_mut(scope, |ctx| {
        ctx.remove_effect(name_filter.as_deref(), index, no_callbacks)
    })?;
    Ok(Value::Bool(removed))
}

fn get_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("GetEffect", args) {
        return result;
    }
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = effect_name_filter("GetEffect", &args[0])?;

    let scope = determine_scope_from_state(&args[1])?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let desired_index = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => *value as usize,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: index argument must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let query = match args.get(3) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for query, got {}",
                other.type_name()
            )))
        }
    };

    let max_priority = match args.get(4) {
        Some(Value::Int(value)) if *value >= 0 => Some(*value),
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: max priority must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for max priority, got {}",
                other.type_name()
            )))
        }
    };

    let found = match name_filter {
        // Name/wildcard given: find by name and index
        // (C4Script.cpp:5471-5472 -> C4Effect::Get(szName, iIndex,...),
        // wildcard compare at C4Effect.cpp:229).
        Some(filter) => effects
            .iter()
            .filter(|effect| s_wildcard_match_ex(&effect.name, filter))
            .filter(|effect| {
                max_priority
                    .map(|limit| effect.priority.abs() <= limit)
                    .unwrap_or(true)
            })
            .nth(desired_index),
        // No name: iIndex is the effect NUMBER (C4Script.cpp:5474-5475 ->
        // C4Effect::Get(iNumber, fIncludeDead=true), C4Effect.cpp:240-256).
        None => effects
            .iter()
            .find(|effect| effect.number == i32::try_from(desired_index).unwrap_or(i32::MAX))
            .filter(|effect| {
                max_priority
                    .map(|limit| effect.priority.abs() <= limit)
                    .unwrap_or(true)
            }),
    };

    Ok(found
        .map(|effect| match query {
            // 0: number (C4Script.cpp:5481 `C4VInt(pEffect->iNumber)`)
            0 => Value::Int(effect.number),
            1 => Value::String(effect.name.clone()),
            2 => Value::Int(effect.priority),
            3 => Value::Int(effect.interval),
            4 => effect.command_target.map(Value::Int).unwrap_or(Value::Nil),
            5 => effect
                .command_id
                .as_ref()
                .map(|id| Value::String(id.clone()))
                .unwrap_or(Value::Nil),
            6 => Value::Int(effect.timer),
            _ => build_effect_value(effect),
        })
        .unwrap_or(Value::Nil))
}

fn get_effect_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("GetEffectCount", args) {
        return result;
    }
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffectCount expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = effect_name_filter("GetEffectCount", &args[0])?;

    let scope = determine_scope_from_state(&args[1])?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let max_priority = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => Some(*value),
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffectCount: max priority must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffectCount: expected int for max priority, got {}",
                other.type_name()
            )))
        }
    };

    let count = effects
        .iter()
        .filter(|effect| {
            if let Some(filter) = name_filter {
                // C4Effect::GetCount wildcard-compares names (C4Effect.cpp:263).
                if !s_wildcard_match_ex(&effect.name, filter) {
                    return false;
                }
            }
            if let Some(limit) = max_priority {
                if effect.priority.abs() > limit {
                    return false;
                }
            }
            true
        })
        .count();

    let count = i32::try_from(count).unwrap_or(i32::MAX);
    Ok(Value::Int(count))
}

fn effect_var(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "EffectVar expects at least 3 arguments: index, state, and number",
        ));
    }

    let var_index = match &args[0] {
        Value::Int(value) if *value >= 0 => *value as usize,
        Value::Int(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "EffectVar: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;

    let effect_number = match &args[2] {
        Value::Int(value) if *value > 0 => *value as usize,
        Value::Int(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "EffectVar: expected positive int for number, got {}",
                other.type_name()
            )))
        }
    };

    let new_value = args.get(3).map(value_to_effect_var);

    let context_value = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(None);
        };

        match context.scope_mut(scope) {
            Ok(stack) => Ok(stack.effect_var(effect_number, var_index, new_value.clone())),
            Err(_) => Ok(None),
        }
    })?;

    if let Some(value) = context_value {
        return Ok(effect_var_to_value(&value));
    }

    if new_value.is_some() {
        return Err(RuntimeError::new(
            "EffectVar: setting variables requires an active engine context",
        ));
    }

    // FnEffectVar resolves the effect BY NUMBER (C4Effect::Get(iNumber,
    // true), C4Script.cpp:5583); snapshot proplists carry positional
    // stand-in numbers from extract_effects_from_state.
    let effects = extract_effects_from_state(&args[1])?;
    let number = i32::try_from(effect_number).unwrap_or(i32::MAX);
    Ok(effects
        .iter()
        .find(|effect| effect.number == number)
        .map(|effect| effect_var_to_value(&effect.var(var_index)))
        .unwrap_or(Value::Nil))
}

fn random(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_RUST_RNG_TRACE").is_ok() {
        let context = HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context.object_context().map(|object| {
                    format!("{} {}", object.id().as_u64(), object.effective_action_name())
                })
            })
        });
        let frame = ENVIRONMENT_CONTEXT.with(|cell| {
            cell.borrow().as_ref().map(|context| context.frame).unwrap_or(0)
        });
        if (19..=21).contains(&frame) {
            tracing::warn!(?args, ?context, frame, "RNDCALL");
        }
    }
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Random expects at most 1 argument: upper exclusive bound",
        ));
    }

    // FnRandom's int parameter follows the C4AulExec conversion rules
    // (C4AulExec.cpp:1364-1396): a missing/nil/bool argument converts —
    // Random(GetActMapVal(...)) with a missing action is Random(0) in
    // C++. The count++ happens even for range 0 (C4Random.h:43), and a
    // negative range goes through the unsigned modulo like C++'s usual
    // arithmetic conversions — both live in LcgRng::random.
    let range = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        Value::Bool(flag) => i32::from(*flag),
        other => {
            return Err(RuntimeError::new(format!(
                "Random: expected int for range, got {}",
                other.type_name()
            )))
        }
    };

    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("Random: host context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        let value = rng.random(range);
        Ok(Value::Int(value))
    })
}

fn sound(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("Sound expects at least 1 argument: name"));
    }

    let name = match &args[0] {
        Value::String(value) if !value.is_empty() => value.clone(),
        Value::Nil => return Ok(Value::Bool(true)),
        other => {
            return Err(RuntimeError::new(format!(
                "Sound: expected string for name, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let global = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "Sound", "global")?;
        index += 1;
        flag
    } else {
        false
    };

    let object_value = if let Some(arg) = args.get(index) {
        index += 1;
        Some(arg)
    } else {
        None
    };

    let level = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sound: expected int for level, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        0
    };

    if let Some(Value::Int(_)) | Some(Value::Nil) = args.get(index) {
        index += 1;
    } else if let Some(other) = args.get(index) {
        return Err(RuntimeError::new(format!(
            "Sound: expected int or nil for at_player, got {}",
            other.type_name()
        )));
    }

    let loop_flag = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sound: expected int for loop, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        0
    };

    let multiple = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "Sound", "multiple")?;
        index += 1;
        flag
    } else {
        false
    };

    let custom_falloff = if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) if *value > 0 => Some(*value),
            Value::Int(_) | Value::Nil => None,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sound: expected int for custom_falloff, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        None
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(true)),
        };

        let mut target_id = if let Some(value) = object_value {
            parse_object_reference_argument(value, "Sound", "object")?
        } else {
            None
        };

        if global {
            target_id = None;
        } else if target_id.is_none() {
            target_id = context.object_context().map(|object| object.id());
        }

        if loop_flag < 0 {
            context.audio_mut().stop_sound(&name, target_id);
            return Ok(Value::Bool(true));
        }

        if level < 0 {
            return Ok(Value::Bool(true));
        }

        let mut volume = level;
        if volume == 0 || volume > 100 {
            volume = 100;
        }
        let volume = volume.clamp(0, 100) as u8;
        let looped = loop_flag > 0;
        let custom_falloff = custom_falloff.filter(|value| *value > 0);

        let audio = context.audio_mut();
        if looped && !multiple && audio.is_looping(&name, target_id) {
            return Ok(Value::Bool(true));
        }

        audio.play_sound(&name, target_id, volume, looped, multiple, custom_falloff);
        Ok(Value::Bool(true))
    })
}

fn sound_level(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "SoundLevel expects at least 2 arguments: name and level",
        ));
    }

    let name = match &args[0] {
        Value::String(value) if !value.is_empty() => value.clone(),
        Value::Nil => return Ok(Value::Bool(true)),
        other => {
            return Err(RuntimeError::new(format!(
                "SoundLevel: expected string for name, got {}",
                other.type_name()
            )))
        }
    };

    let level = match &args[1] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SoundLevel: expected int for level, got {}",
                other.type_name()
            )))
        }
    };

    let object_arg = args.get(2);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(true)),
        };

        let mut target_id = if let Some(value) = object_arg {
            parse_object_reference_argument(value, "SoundLevel", "object")?
        } else {
            None
        };

        if target_id.is_none() {
            target_id = context.object_context().map(|object| object.id());
        }

        let audio = context.audio_mut();
        if level <= 0 {
            audio.stop_sound(&name, target_id);
            return Ok(Value::Bool(true));
        }

        let volume = level.clamp(0, 100) as u8;
        let existed = audio.set_volume(&name, target_id, volume, None);
        if !existed {
            audio.play_sound(&name, target_id, volume, true, false, None);
        }
        Ok(Value::Bool(true))
    })
}

fn set_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetGravity expects 1 argument: gravity"));
    }

    let gravity = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetGravity: expected int or nil for gravity, got {}",
                other.type_name()
            )))
        }
    };

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetGravity requires an active engine context"))?
            .clone();
        context.set_gravity(gravity);
        Ok(Value::Nil)
    })
}

fn get_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetGravity does not accept any arguments",
        ));
    }

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetGravity requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.gravity()))
    })
}

fn set_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetWind expects 1 argument: wind"));
    }

    let wind = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetWind: expected int or nil for wind, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetWind requires an active engine context"))?
            .clone();
        context.set_wind(wind);
        Ok(Value::Nil)
    })
}

fn get_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "GetWind expects at most 3 arguments: x, y, global",
        ));
    }

    for (index, arg) in args.iter().enumerate() {
        match arg {
            Value::Int(_) | Value::Nil => {}
            Value::Bool(_) if index == 2 => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "GetWind: unexpected argument type {} at position {}",
                    other.type_name(),
                    index + 1
                )))
            }
        }
    }

    let wind = ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetWind requires an active engine context"))?
            .clone();
        Ok::<i32, RuntimeError>(context.wind_force())
    })?;

    // Global form (FnGetWind, C4Script.cpp:3001-3004).
    let global = match args.get(2) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Int(value)) => *value != 0,
        _ => false,
    };
    if global {
        return Ok(Value::Int(wind));
    }

    // Positional form: object-relative GBackWind — zero on tunnel
    // background (C4Script.cpp:3005-3007; C4Wrappers.h:189-192).
    let local_x = match args.first() {
        Some(Value::Int(value)) => *value,
        _ => 0,
    };
    let local_y = match args.get(1) {
        Some(Value::Int(value)) => *value,
        _ => 0,
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(wind));
        };
        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }
        let in_tunnel = context
            .landscape_ref()
            .map(|landscape| landscape.is_tunnel_at(global_x, global_y))
            .unwrap_or(false);
        Ok(Value::Int(if in_tunnel { 0 } else { wind }))
    })
}

// Mathematical host functions

fn abs_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("Abs expects 1 argument: value"));
    }

    match &args[0] {
        Value::Int(value) => Ok(Value::Int(value.abs())),
        Value::Nil => Ok(Value::Int(0)),
        other => Err(RuntimeError::new(format!(
            "Abs: expected int, got {}",
            other.type_name()
        ))),
    }
}

fn min_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::new("Min expects 2 arguments: val1, val2"));
    }

    let val1 = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Min: expected int for first argument, got {}",
                other.type_name()
            )))
        }
    };

    let val2 = match &args[1] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Min: expected int for second argument, got {}",
                other.type_name()
            )))
        }
    };

    Ok(Value::Int(val1.min(val2)))
}

fn max_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::new("Max expects 2 arguments: val1, val2"));
    }

    let val1 = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Max: expected int for first argument, got {}",
                other.type_name()
            )))
        }
    };

    let val2 = match &args[1] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Max: expected int for second argument, got {}",
                other.type_name()
            )))
        }
    };

    Ok(Value::Int(val1.max(val2)))
}

fn sqrt_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("Sqrt expects 1 argument: value"));
    }

    let value = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Sqrt: expected int, got {}",
                other.type_name()
            )))
        }
    };

    // C++ returns 0 for negative values
    if value < 0 {
        return Ok(Value::Int(0));
    }

    // C++ implementation does: sqrt, then adjusts for rounding
    let result = (value as f64).sqrt() as i32;
    Ok(Value::Int(result))
}

/// FnGetMass (C4Script.cpp:1148-1158): with an id, the DEF mass; else the
/// object's Mass = max(Def->Mass * Con / FullCon, 1)
/// (C4Object.cpp:188; OwnMass/contents mass unmodeled). Nil without both.
fn get_mass(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = parse_definition_argument(args.get(1), "GetMass")?;
    if let Some(definition) = definition {
        return HOST_CONTEXT.with(|cell| {
            Ok(cell
                .borrow()
                .as_ref()
                .and_then(|context| context.world.definition_metadata(&definition))
                .map(|metadata| Value::Int(metadata.mass))
                .unwrap_or(Value::Nil))
        });
    }
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetMass", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let id = target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(id) = id else {
            return Ok(Value::Nil);
        };
        // UpdateMass: Mass = max((Def->Mass + OwnMass) * Con / FullCon, 1)
        // (C4Object.cpp:497-500); the active scope has the freshest OwnMass.
        let scope_own_mass = context
            .object_context()
            .filter(|object| object.id() == id)
            .map(|object| object.own_mass());
        Ok(context
            .get_world_object(id)
            .and_then(|object| {
                let metadata = context.world.definition_metadata(object.definition_id())?;
                let state = object.full_state();
                let construction = state
                    .map(|state| state.construction)
                    .unwrap_or(crate::FULL_CON);
                let own_mass = scope_own_mass
                    .or_else(|| state.map(|state| state.own_mass))
                    .unwrap_or(0);
                Some(Value::Int(
                    ((metadata.mass.saturating_add(own_mass)).saturating_mul(construction)
                        / crate::FULL_CON)
                        .max(1),
                ))
            })
            .unwrap_or(Value::Nil))
    })
}

/// FnMakeCrewMember (C4Script.cpp:2164-2168) -> C4Player::MakeCrewMember
/// (C4Player.cpp:1167-1215): valid player + CrewMember def required; adds
/// to the crew and fires OnJoinCrew(player). NOT modeled (PORT_STATUS):
/// the crew-info GetIdle/New assignment (a ClonkNames RNG draw in C++),
/// PlrViewRange, and Controller — the port keys crew off Owner, so the
/// owner is set to the player instead.
/// FnGrabObjectInfo (C4Script.cpp:2170-2176) -> C4Object::GrabInfo
/// (C4Object.cpp:5696-5726): `pTo` (default: the caller) takes pFrom's
/// info section, retires its own, and re-registers as crew. The port
/// keys "has an info" off the crew flag; the transferred payload is the
/// crew flag, the donor's portrait source and the info's permanent
/// physicals (the GetPhysical info fallback, C4Object.cpp:2118-2134).
/// The name/rank/experience payload of C4ObjectInfo is not modeled
/// (documented gap: GetName reads definition metadata) — GoldRush TRPR
/// Recruitment, Trapper.c4d/Script.c:19-25.
fn grab_object_info(args: &[Value]) -> Result<Value, RuntimeError> {
    let from = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GrabObjectInfo",
        "from",
    )?;
    let Some(from) = from else {
        return Ok(Value::Bool(false));
    };
    let to = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "GrabObjectInfo",
        "to",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let active = context.object_context().map(|object| object.id());
        let Some(to) = to.or(active) else {
            return Ok(Value::Bool(false));
        };
        if from == to {
            // own info: success (C4Object.cpp:5701)
            return Ok(Value::Bool(true));
        }
        // Materialize scopes for both sides (C++ mutates the live objects).
        if !context.ensure_object_scope(from) || !context.ensure_object_scope(to) {
            return Ok(Value::Bool(false));
        }
        // only if the other object has an info (C4Object.cpp:5703)
        let (from_has_info, donor_physical) = context
            .object_scope(from)
            .map(|scope| (scope.crew_member, scope.info_physical))
            .unwrap_or((false, None));
        if !from_has_info {
            return Ok(Value::Bool(false));
        }
        // the grabbed info carries the donor's portrait
        // (C4Object.cpp:5715 Info transfer; portrait rides the info)
        let donor_portrait = context
            .get_world_object(from)
            .and_then(|object| {
                object
                    .full_state()
                    .map(|state| state.portrait_source.clone())
            })
            .flatten()
            .or_else(|| {
                context
                    .get_world_object(from)
                    .map(|object| object.definition_id().to_string())
            });
        // the donor loses its info/crew slot — and with it the info's
        // permanent physicals (`pFrom->ClearInfo(pFrom->Info)`,
        // C4Object.cpp:5710-5715)
        if let Some(scope) = context.object_scope_mut(from) {
            scope.set_crew_member(false);
            scope.info_physical = None;
            scope.record_physicals();
        }
        // the grabber takes the info wholesale: crew registration, the
        // donor's portrait and its permanent physicals
        // (C4Object.cpp:5715-5723; GetPhysical's info fallback,
        // C4Object.cpp:2118-2134)
        if let Some(scope) = context.object_scope_mut(to) {
            scope.set_crew_member(true);
            scope.info_physical = donor_physical;
            scope.record_physicals();
            if let Some(portrait) = donor_portrait {
                scope.set_portrait_source(portrait);
            }
        }
        Ok(Value::Bool(true))
    })
}

fn make_crew_member(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "MakeCrewMember",
        "obj",
    )?;
    let player = parse_optional_i32(args.get(1), "MakeCrewMember", "player")?.unwrap_or(0);
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "MakeCrewMember",
                &[Value::Nil, Value::Int(player)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    let joined = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(false);
        };
        if context.player_state(player).is_none() {
            return Ok(false); // ValidPlr (C4Script.cpp:2166)
        }
        let Some(id) = context.object_context().map(|object| object.id()) else {
            return Ok(false);
        };
        let crew_def = context
            .get_world_object(id)
            .and_then(|object| context.world.definition_metadata(object.definition_id()))
            .map(|metadata| metadata.crew_member)
            .unwrap_or(false);
        if !crew_def {
            return Ok(false); // Def->CrewMember required (C4Player.cpp:1170)
        }
        // C4Player::MakeCrewMember info selection (C4Player.cpp:1180-1199):
        // the gate is `!pObj->Info` — an object that already carries a crew
        // info skips; an idle info skips the New; otherwise
        // C4ObjectInfoList::New draws the name Random over the def's
        // ClonkNames (or the game standard names) — a synced ledger draw
        // that must fire INSIDE this call.
        let already_crew = context
            .object_context()
            .map(|object| context.world.crew_ranks.contains_key(&object.id().as_u64()))
            .unwrap_or(false);
        if !already_crew {
            let definition_id = context
                .object_context()
                .and_then(|object| object.definition_id.clone());
            if let Some(definition_id) = definition_id {
                let mut idle = context.world.idle_crew_counts.borrow_mut();
                let key = (player, definition_id.clone());
                match idle.get_mut(&key) {
                    Some(count) if *count > 0 => {
                        *count -= 1;
                    }
                    _ => {
                        let newlines = context
                            .world
                            .definition_metadata(&definition_id)
                            .and_then(|metadata| metadata.clonk_name_newlines)
                            .unwrap_or(context.world.standard_name_newlines);
                        drop(idle);
                        draw_context_random(newlines)?;
                    }
                }
            }
        }
        let Some(object) = context.object_context_mut() else {
            return Ok(false);
        };
        object.set_crew_member(true);
        object.set_owner(player);
        // The new crew member gets an info whose physicals resolve at
        // once (C4Player::MakeCrewMember → C4ObjectInfoList::New;
        // C4Object::Init `if (Alive) Energy = GetPhysical()->Energy`,
        // C4Object.cpp:192). With FairCrew ON (the LegacyClonk default)
        // the def physicals promote to RankByExperience(strength)
        // (C4Def.cpp:860-874; PromotionUpdate Energy = max(def,
        // (50+5*rank)*C4MaxPhysical/100, C4InfoCore.cpp:207-213) — the
        // same resolution the join path uses. Without it the fresh COWB
        // the GoldRush Trapper recruits-and-grabs carries NO info
        // physicals, and the grab downgraded the Trapper's max energy.
        let mut promoted = object.definition_physical;
        let rank = if crate::USE_FAIR_CREW {
            crate::fair_crew_rank(crate::FAIR_CREW_STRENGTH)
        } else {
            0
        };
        promoted.energy = promoted
            .energy
            .max((50 + 5 * rank.clamp(0, 10)) * (crate::C4_MAX_PHYSICAL / 100));
        object.info_physical = Some(promoted);
        object.record_physicals();
        if object.alive() {
            object.set_energy(promoted.energy);
        }
        Ok(true)
    })?;
    if joined {
        // PSF_OnJoinCrew resolves to the script function "Recruitment"
        // (C4Script.h:107 `#define PSF_OnJoinCrew "~Recruitment"`), fired
        // inside MakeCrewMember (C4Player.cpp:1206-1209); errors pass
        // like any script call.
        if let Some(active) = active {
            if let Some(result) =
                call_world_object_script_function(active, "Recruitment", &[Value::Int(player)])
            {
                result?;
            }
        }
    }
    Ok(Value::Bool(joined))
}

/// FnSetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass, then
/// UpdateMass; foreign targets mutate in their own scope.
fn set_mass(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = parse_optional_i32(args.first(), "SetMass", "mass")?.unwrap_or(0);
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "SetMass", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(target, "SetMass", &[Value::Int(value)]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(id) = context.object_context().map(|object| object.id()) else {
            return Ok(Value::Bool(false));
        };
        let def_mass = context
            .get_world_object(id)
            .and_then(|object| context.world.definition_metadata(object.definition_id()))
            .map(|metadata| metadata.mass)
            .unwrap_or(0);
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        object.set_own_mass(value - def_mass);
        Ok(Value::Bool(true))
    })
}

/// FnMod (C4Script.cpp:3219-3223): truncated `%`, zero divisor yields 0.
fn modulo(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = parse_optional_i32(args.first(), "Mod", "value")?.unwrap_or(0);
    let divisor = parse_optional_i32(args.get(1), "Mod", "divisor")?.unwrap_or(0);
    Ok(Value::Int(if divisor == 0 {
        0
    } else {
        value.wrapping_rem(divisor)
    }))
}

/// FnC4Id (C4Script.cpp:2396-2399): string to C4ID; empty/nil is C4ID_None
/// (0 — falsy, like C4Id("") in C++).
fn c4_id(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "C4Id", "id")?;
    Ok(match name {
        Some(name) if !name.is_empty() => Value::C4Id(name),
        _ => Value::Int(0),
    })
}

fn pow_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::new("Pow expects 2 arguments: base, exponent"));
    }

    let base = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Pow: expected int for base, got {}",
                other.type_name()
            )))
        }
    };

    let exponent = match &args[1] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Pow: expected int for exponent, got {}",
                other.type_name()
            )))
        }
    };

    if exponent < 0 {
        return Ok(Value::Int(0)); // Match C++ behavior for negative exponents
    }

    Ok(Value::Int(base.pow(exponent as u32)))
}

fn bound_by_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 3 {
        return Err(RuntimeError::new(
            "BoundBy expects 3 arguments: value, min, max",
        ));
    }

    let value = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for value, got {}",
                other.type_name()
            )))
        }
    };

    let range1 = match &args[1] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for range1, got {}",
                other.type_name()
            )))
        }
    };

    let range2 = match &args[2] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for range2, got {}",
                other.type_name()
            )))
        }
    };

    // BoundBy clamps value between range1 and range2 (order doesn't matter)
    let min = range1.min(range2);
    let max = range1.max(range2);
    Ok(Value::Int(value.clamp(min, max)))
}

fn sin_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 3 {
        return Err(RuntimeError::new(
            "Sin expects 1-3 arguments: angle, radius, precision",
        ));
    }

    let angle = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Sin: expected int for angle, got {}",
                other.type_name()
            )))
        }
    };

    let radius = if args.len() > 1 {
        match &args[1] {
            Value::Int(v) => *v,
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sin: expected int for radius, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        1
    };

    let precision = if args.len() > 2 {
        match &args[2] {
            Value::Int(v) => {
                if *v == 0 {
                    1
                } else {
                    *v
                }
            }
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sin: expected int for precision, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        1
    };

    // C++ implementation: modulo to prevent overflow, convert to radians
    let angle_mod = angle % (360 * precision);
    let angle_radians = (angle_mod as f64 / precision as f64) * std::f64::consts::PI / 180.0;
    let result = (angle_radians.sin() * radius as f64).round() as i32;
    Ok(Value::Int(result))
}

fn cos_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 3 {
        return Err(RuntimeError::new(
            "Cos expects 1-3 arguments: angle, radius, precision",
        ));
    }

    let angle = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Cos: expected int for angle, got {}",
                other.type_name()
            )))
        }
    };

    let radius = if args.len() > 1 {
        match &args[1] {
            Value::Int(v) => *v,
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Cos: expected int for radius, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        1
    };

    let precision = if args.len() > 2 {
        match &args[2] {
            Value::Int(v) => {
                if *v == 0 {
                    1
                } else {
                    *v
                }
            }
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Cos: expected int for precision, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        1
    };

    // C++ implementation: modulo to prevent overflow, convert to radians
    let angle_mod = angle % (360 * precision);
    let angle_radians = (angle_mod as f64 / precision as f64) * std::f64::consts::PI / 180.0;
    let result = (angle_radians.cos() * radius as f64).round() as i32;
    Ok(Value::Int(result))
}

fn set_temperature(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetTemperature expects 1 argument: temperature",
        ));
    }

    let temperature = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetTemperature: expected int or nil for temperature, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetTemperature requires an active engine context"))?
            .clone();
        context.set_temperature(temperature);
        Ok(Value::Nil)
    })
}

fn get_temperature(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetTemperature does not accept any arguments",
        ));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetTemperature requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.ambient_temperature()))
    })
}

fn set_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetClimate expects 1 argument: climate"));
    }

    let climate = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetClimate: expected int or nil for climate, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetClimate requires an active engine context"))?
            .clone();
        context.set_climate(climate);
        Ok(Value::Nil)
    })
}

fn get_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetClimate does not accept any arguments",
        ));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetClimate requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.climate()))
    })
}

fn get_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetEnergy expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetEnergy", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(energy_to_script_value(object.energy())));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(energy_to_script_value(other.energy())));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(energy_to_script_value(object.energy())))
    })
}

/// FnGetBreath (C4Script.cpp:1142-1146): `100 * Breath / C4MaxPhysical`.
/// Breath ticks in ExecLife; the world snapshot (pre-frame) stands in for
/// the live value — the scaled result changes at 1/1000th granularity.
fn get_breath(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetBreath", "target")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        match context
            .get_world_object(target)
            .and_then(|object| object.full_state().map(|state| state.breath))
        {
            Some(breath) => Ok(Value::Int(
                (100i64 * i64::from(breath) / i64::from(LEGACY_MAX_PHYSICAL)) as i32,
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// FnGetName (C4Script.cpp:992-1005): the definition Name for an id
/// argument; the object's name otherwise (custom object names are
/// unmodeled — the definition name stands in, matching fresh objects).
fn get_name(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetName", "target")?;
    let def_id = match args.get(index) {
        Some(Value::C4Id(id)) => Some(id.clone()),
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        _ => None,
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let definition = if let Some(def_id) = def_id {
            Some(def_id)
        } else {
            let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
            target.and_then(|target| {
                context
                    .get_world_object(target)
                    .map(|object| object.definition_id().to_string())
            })
        };
        Ok(definition
            .and_then(|id| context.definition_metadata(&id))
            .map(|metadata| Value::String(metadata.name.clone()))
            .unwrap_or(Value::Nil))
    })
}

fn get_con(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetCon expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetCon", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(construction_to_script_value(
                        object.construction(),
                    )));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(construction_to_script_value(
                    other.construction(),
                )));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(construction_to_script_value(
            object.construction(),
        )))
    })
}

/// A physical name argument (`FnStringPar`): None for Nil/absent/empty.
fn physical_name_argument(
    args: &[Value],
    index: usize,
    fn_name: &str,
) -> Result<Option<String>, RuntimeError> {
    match args.get(index) {
        Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.clone())),
        Some(Value::String(_)) | Some(Value::Nil) | None => Ok(None),
        Some(other) => Err(RuntimeError::new(format!(
            "{fn_name}: expected string for physical name, got {}",
            other.type_name()
        ))),
    }
}

fn int_argument(args: &[Value], index: usize, fn_name: &str) -> Result<i32, RuntimeError> {
    match args.get(index) {
        Some(Value::Int(value)) => Ok(*value),
        Some(Value::Nil) | None => Ok(0),
        Some(other) => Err(RuntimeError::new(format!(
            "{fn_name}: expected int, got {}",
            other.type_name()
        ))),
    }
}

/// `FnGetPhysical` (C4Script.cpp:638-688): `GetPhysical(name, mode, obj,
/// id)`. The def form reads the definition's `[Physical]` section; object
/// reads resolve against this object only (the host model does not mutate or
/// read foreign objects' physicals yet).
fn get_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(name) = physical_name_argument(args, 0, "GetPhysical")? else {
        return Ok(Value::Nil);
    };
    let mode = int_argument(args, 1, "GetPhysical")?;
    let target_id = args
        .get(2)
        .map(|arg| parse_object_reference_argument(arg, "GetPhysical", "target"))
        .transpose()?
        .flatten();
    let definition_id = match args.get(3) {
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        _ => None,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        // No object given: a def id reads the definition physicals
        // (C4Script.cpp:644-653).
        if target_id.is_none() {
            if let Some(definition_id) = definition_id {
                return Ok(context
                    .world
                    .definition_metadata(&definition_id)
                    .and_then(|metadata| metadata.physical.value_by_name(&name))
                    .map(Value::Int)
                    .unwrap_or(Value::Nil));
            }
        }
        let Some(object) = context.object_context() else {
            return Ok(Value::Nil);
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }
        Ok(object
            .get_physical(&name, mode)
            .map(Value::Int)
            .unwrap_or(Value::Nil))
    })
}

/// `FnSetPhysical` (C4Script.cpp:557-601): `SetPhysical(name, value, mode,
/// obj)`.
fn set_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(name) = physical_name_argument(args, 0, "SetPhysical")? else {
        return Ok(Value::Bool(false));
    };
    let value = int_argument(args, 1, "SetPhysical")?;
    let mode = int_argument(args, 2, "SetPhysical")?;
    let target_id = args
        .get(3)
        .map(|arg| parse_object_reference_argument(arg, "SetPhysical", "target"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetPhysical requires an active engine context"))?;
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(object.set_physical(&name, value, mode)))
    })
}

/// `FnTrainPhysical` (C4Script.cpp:603-611): `TrainPhysical(name, by, max,
/// obj)`.
fn train_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(name) = physical_name_argument(args, 0, "TrainPhysical")? else {
        return Ok(Value::Bool(false));
    };
    let train_by = int_argument(args, 1, "TrainPhysical")?;
    let max_train = int_argument(args, 2, "TrainPhysical")?;
    let target_id = args
        .get(3)
        .map(|arg| parse_object_reference_argument(arg, "TrainPhysical", "target"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("TrainPhysical requires an active engine context"))?;
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(
            object.train_physical(&name, train_by, max_train),
        ))
    })
}

/// `FnResetPhysical` (C4Script.cpp:613-636): `ResetPhysical(obj, name)` —
/// the object comes FIRST in this one.
fn reset_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "ResetPhysical", "target"))
        .transpose()?
        .flatten();
    let name = physical_name_argument(args, 1, "ResetPhysical")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("ResetPhysical requires an active engine context"))?;
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(object.reset_physical(name.as_deref())))
    })
}

fn do_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "DoEnergy expects at least 1 argument: change",
        ));
    }

    let change = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoEnergy: expected int or nil for change, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Object(_) | Value::Proplist(_) => {
                target_id = object_id_from_value(arg);
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            Value::Int(value) if *value == 0 => {
                index += 1;
            }
            Value::Int(value) if *value > 0 => {
                target_id = Some(ObjectId::new(*value as u64));
                index += 1;
            }
            _ => {}
        }
    }

    let mut exact = false;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Bool(flag) => {
                exact = *flag;
                index += 1;
            }
            Value::Int(value) => {
                exact = *value != 0;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected bool, int, or nil for exact flag, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let mut eng_type = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                eng_type = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for cause, got {}",
                    other.type_name()
                )))
            }
        }
    }
    // C4FxCall_EngScript default (C4Script.cpp:495).
    if eng_type == 0 {
        eng_type = crate::C4FX_CALL_ENG_SCRIPT;
    }

    let mut caused_by_plus_one = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                caused_by_plus_one = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for caused by, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoEnergy: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoEnergy requires an active engine context"))?;
        // iCausedBy = iCausedByPlusOne - 1, else the CALLER's controller
        // (C4Script.cpp:496-497) — resolved in the caller's scope.
        let caused_by = if caused_by_plus_one != 0 {
            caused_by_plus_one - 1
        } else {
            context
                .object_context()
                .map(|object| object.controller())
                .unwrap_or(OWNER_NONE)
        };
        // `if (!pObj) pObj = cthr->Obj` is only the local-call default
        // (C4Script.cpp:494) — a named target may be FOREIGN.
        let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id())) else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        stage_energy_loss_cause(context, target, change, eng_type, caused_by);
        let Some(scope) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        scope.adjust_energy(change, exact);
        Ok(Value::Bool(true))
    })
}

/// The kill-trace mark of C4Object::DoEnergy (C4Object.cpp:1351-1353):
/// negative changes (and object hits even at zero) record the causing
/// player, with the UpdatLastEnergyLossCause guard (:1369-1378) applied
/// at call time against the freshest tracked value.
fn stage_energy_loss_cause(
    context: &mut EffectHostContext,
    target: ObjectId,
    change: i32,
    eng_type: i32,
    caused_by: i32,
) {
    if change >= 0 && eng_type != crate::C4FX_CALL_ENG_OBJ_HIT {
        return;
    }
    let tracked = context
        .object_scope(target)
        .and_then(|scope| scope.pending_update.energy_loss_cause)
        .or_else(|| {
            context
                .get_world_object(target)
                .map(|object| object.last_energy_loss_cause)
        })
        .unwrap_or(OWNER_NONE);
    let controller = context
        .object_scope(target)
        .map(|scope| scope.controller())
        .unwrap_or(OWNER_NONE);
    if caused_by != controller || tracked < 0 {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.pending_update.energy_loss_cause = Some(caused_by);
        }
    }
}

fn do_con(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "DoCon expects at least 1 argument: change",
        ));
    }

    let change_percent = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoCon: expected int or nil for change, got {}",
                other.type_name()
            )))
        }
    };

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(1) {
        target_id = parse_object_reference_argument(arg, "DoCon", "target")?;
    }

    if args.len() > 2 {
        return Err(RuntimeError::new(
            "DoCon: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoCon requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        let delta = construction_delta_from_percent(change_percent);
        object.adjust_construction(delta);
        Ok(Value::Bool(true))
    })
}

fn do_damage(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "DoDamage expects at least 1 argument: change",
        ));
    }

    let change = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoDamage: expected int or nil for change, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Object(_) | Value::Proplist(_) => {
                target_id = object_id_from_value(arg);
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            Value::Int(value) if *value == 0 => {
                index += 1;
            }
            Value::Int(value) if *value > 0 => {
                target_id = Some(ObjectId::new(*value as u64));
                index += 1;
            }
            _ => {}
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for damage type, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let mut caused_by_plus_one = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                caused_by_plus_one = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for caused by, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoDamage: additional arguments are not supported",
        ));
    }

    let staged = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoDamage requires an active engine context"))?;
        // iCausedBy = iCausedByPlusOne - 1, else the CALLER's controller
        // (C4Script.cpp:511) — resolved in the caller's scope.
        let caused_by = if caused_by_plus_one != 0 {
            caused_by_plus_one - 1
        } else {
            context
                .object_context()
                .map(|object| object.controller())
                .unwrap_or(OWNER_NONE)
        };
        // `if (!pObj) pObj = cthr->Obj` is only the local-call default
        // (C4Script.cpp:510) — a named target may be FOREIGN.
        let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id())) else {
            return Ok(None);
        };
        if !context.ensure_object_scope(target) {
            return Ok(None);
        }
        let Some(scope) = context.object_scope_mut(target) else {
            return Ok(None);
        };
        // Damage = max(Damage + iChange, 0) (C4Object.cpp:1288).
        scope.adjust_damage(change);
        Ok(Some((target, caused_by)))
    })?;
    let Some((target, caused_by)) = staged else {
        return Ok(Value::Bool(false));
    };
    // The Damage engine call after the stat write (PSF_Damage "~Damage",
    // C4Object.cpp:1290 — fail-safe exec, errors log and continue).
    // NOT modeled: the non-living Fx*Damage effects-first hook
    // (C4Object.cpp:1282-1286) — the host scope path has no effect
    // DoDamage dispatch (same gap as DoEnergy's living hook).
    if let Some(Err(error)) = call_world_object_own_function(
        target,
        "Damage",
        &[Value::Int(change), Value::Int(caused_by)],
    ) {
        tracing::warn!(
            %error,
            "script error in Damage; continuing like the C++ fail-safe exec"
        );
    }
    Ok(Value::Bool(true))
}

fn set_action(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_DEBUG_CHBM").is_ok() {
        tracing::warn!(?args, "OSA SetAction");
    }
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetAction expects at least 1 argument: action name",
        ));
    }

    let action_name = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAction: expected string or nil for action name, got {}",
                other.type_name()
            )))
        }
    };

    // FnSetAction (C4Script.cpp:747-753): (szAction, pTarget, pTarget2,
    // fDirect) — the objects are the ACTION's targets
    // (SetActionByName(..., pTarget, pTarget2)). fDirect (skip the phase
    // reset) is accepted and ignored for now (PORT_STATUS).
    let target1 = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetAction", "target"))
        .transpose()?
        .flatten();
    let target2 = args
        .get(2)
        .map(|arg| parse_object_reference_argument(arg, "SetAction", "target2"))
        .transpose()?
        .flatten();
    if args.len() > 4 {
        return Err(RuntimeError::new(format!(
            "SetAction: expected at most 4 arguments, got {}",
            args.len()
        )));
    }

    let name = match action_name {
        Some(name) => name,
        None => return Ok(Value::Bool(false)),
    };

    let mut sync_callbacks: Option<(ObjectId, Option<String>, Option<String>)> = None;
    let staged = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetAction requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        let current_action = object.effective_action_name().to_string();
        let blocks_other_actions = object.effective_blocks_other_actions();
        let changed_action = name != current_action;
        if blocks_other_actions && name != current_action {
            return Ok(Value::Bool(false));
        }

        let update = object
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_name(name.clone());
        update.set_force(false);

        // SetActionByName carries the action targets, and C4Object::SetAction
        // assigns them ONLY when non-null (C4Object.cpp:4123-4125:
        // `if (pTarget) Action.Target = pTarget;`) — nil preserves.
        if target1.is_some() {
            object.set_action_target(0, target1);
        }
        if target2.is_some() {
            object.set_action_target(1, target2);
        }
        if changed_action {
            object.reset_action_ticks();
        }

        let procedure_changed = object.update_effective_action(&name);
        if procedure_changed {
            object.reset_action_data();
        }
        // C4Object::SetAction fires the calls with NO same-action gate
        // (C4Object.cpp:4146-4183) — a same-name SetAction re-runs the
        // StartCall once. Marking the update dispatched keeps the fold
        // from ALSO queueing deferred Abort/Start events (C++ has no
        // such queue; leaving same-name staging to the queue made the
        // coach's Driving guard loop forever against stale state).
        if let Some(update) = object.pending_update.action.as_mut() {
            update.callbacks_dispatched = true;
        }
        sync_callbacks = Some((
            object.id(),
            object
                .action_library
                .abort_call_for_action(&current_action)
                .map(str::to_string),
            object
                .action_library
                .start_call_for_action(&name)
                .map(str::to_string),
        ));

        Ok(Value::Bool(true))
    })?;
    // C4Object::SetAction runs the AbortCall for the OLD action and the
    // StartCall for the NEW one SYNCHRONOUSLY inside the call
    // (SetActionByName defaults SAC_StartCall|SAC_AbortCall) — the
    // coach's Drive0 StartCall reads the PRE-SetDir facing for its seat
    // vertex, so deferral changes the result.
    if let Some((id, abort_call, start_call)) = sync_callbacks {
        let depth = SETACTION_DEPTH.with(|d| {
            d.set(d.get() + 1);
            d.get()
        });
        // C++ has no recursion guard here — content terminates because its
        // guards read the LIVE Action.Name (set before StartCall,
        // C4Object.cpp:4116-4152). The rust nested-scope view can go stale
        // across recursion levels (rider Riding vs coach IsStill —
        // PORT_STATUS), so the backstop turns that defect into a log line
        // instead of a freeze. Legitimate content chains are depth <= 3
        // (mount -> RideStill -> Ride).
        if depth > 16 {
            tracing::warn!(?id, "SetAction callback recursion backstop hit");
            SETACTION_DEPTH.with(|d| d.set(d.get() - 1));
            return Ok(staged);
        }
        for callback in [abort_call, start_call].into_iter().flatten() {
            if let Some(Err(error)) = call_world_object_own_function(id, &callback, &[]) {
                tracing::warn!(
                    %error,
                    callback,
                    "SetAction callback error; continuing like the C++ fail-safe exec"
                );
            }
        }
        SETACTION_DEPTH.with(|d| d.set(d.get() - 1));
    }
    Ok(staged)
}

fn set_bridge_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetBridgeActionData expects at least 1 argument: length",
        ));
    }

    let mut param_count = args.len();
    let mut target_id: Option<ObjectId> = None;

    if let Some(Value::Proplist(_)) = args.last() {
        param_count -= 1;
        target_id =
            parse_object_reference_argument(&args[param_count], "SetBridgeActionData", "object")?;
    }

    if param_count == 0 {
        return Err(RuntimeError::new(
            "SetBridgeActionData expects at least 1 argument: length",
        ));
    }

    if param_count > 4 {
        return Err(RuntimeError::new(
            "SetBridgeActionData accepts at most 4 arguments before the object parameter",
        ));
    }

    let length = value_to_i32(&args[0], "SetBridgeActionData", "length")?;
    let move_clonk = if param_count > 1 {
        value_to_bool(&args[1], "SetBridgeActionData", "move_clonk")?
    } else {
        false
    };
    let wall = if param_count > 2 {
        value_to_bool(&args[2], "SetBridgeActionData", "wall")?
    } else {
        false
    };
    let material = if param_count > 3 {
        match &args[3] {
            Value::Nil => -1,
            other => value_to_i32(other, "SetBridgeActionData", "material")?,
        }
    } else {
        -1
    };

    let encoded = encode_bridge_action_data(length, move_clonk, wall, material);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetBridgeActionData requires an active engine context")
        })?;

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if !object.status().is_active() {
            return Ok(Value::Bool(false));
        }

        if object.effective_action_procedure() != ActionProcedure::Bridge {
            return Ok(Value::Bool(false));
        }

        object.set_action_data(encoded);
        Ok(Value::Bool(true))
    })
}

fn set_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetActionData expects at least 1 argument: data",
        ));
    }

    let data = value_to_i32(&args[0], "SetActionData", "data")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetActionData", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetActionData: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetActionData requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if !object.status().is_active() {
            return Ok(Value::Bool(false));
        }

        let procedure = object.effective_action_procedure();
        let mut next_data = data;
        match procedure {
            ActionProcedure::Bridge => {
                next_data = if data < 0 { 0xFF } else { data.min(0xFF) };
            }
            ActionProcedure::Attach => {
                let primary_vertex = (data & 0xFF) as i32;
                let secondary_vertex = data >> 8;
                if primary_vertex >= MAX_VERTEX_COUNT || secondary_vertex >= MAX_VERTEX_COUNT {
                    return Ok(Value::Bool(false));
                }
            }
            _ => {}
        }

        object.set_action_data(next_data);
        Ok(Value::Bool(true))
    })
}

fn set_action_targets(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;

    let (target1, update_target1) = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target1")?;
        index += 1;
        (target, true)
    } else {
        (None, false)
    };

    let (target2, update_target2) = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target2")?;
        index += 1;
        (target, true)
    } else {
        (None, false)
    };

    let mut object_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        object_id = parse_object_reference_argument(arg, "SetActionTargets", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetActionTargets: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetActionTargets requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = object_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if update_target1 {
            object.set_action_target(0, target1);
        }
        if update_target2 {
            object.set_action_target(1, target2);
        }

        Ok(Value::Bool(true))
    })
}

/// FnShiftContents (C4Script.cpp:1784-1797): rotate the contents list so a
/// different item comes first (C4Object::ShiftContents,
/// C4Object.cpp:5728-5752) or bring the first idTarget content to the
/// front (DirectComContents, :5754-5775). The rotation itself is the C++
/// cyclic relink (C4ObjectList.cpp:815-833), applied via
/// ObjectUpdate.contents_front; with fDoCalls the container's
/// ~ControlContents(id) may veto and the new front gets
/// ~Selection(container) with the Grab sound on a falsy return
/// (C4Object.cpp:5760-5767); the menu Refill (:5769-5772) is
/// presentation-only and unmodeled. Documented gaps: CanConcatPictureWith
/// (id/color/graphics/name/overlay stack check, C4Object.cpp) is
/// approximated by DEFINITION ID equality; contents read the frame-start
/// world view (mid-call CreateContents spawns are not visible — the
/// FindContents staleness seam), so ~Selection also fires against the
/// pre-relink view.
fn shift_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_object = match args.first() {
        None | Some(Value::Nil | Value::Int(0)) => None,
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            parse_object_reference_argument(value, "ShiftContents", "target")?
        }
        // C++ par conversion nils a non-object slot -> local call.
        Some(_) => None,
    };
    let shift_back = args
        .get(1)
        .map(|value| value_to_bool(value, "ShiftContents", "shift back"))
        .transpose()?
        .unwrap_or(false);
    let id_target = parse_definition_argument(args.get(2), "ShiftContents")?;
    let do_calls = args
        .get(3)
        .map(|value| value_to_bool(value, "ShiftContents", "do calls"))
        .transpose()?
        .unwrap_or(false);

    // FnShiftContents' pObj may name ANOTHER container (C4Script.cpp:1786
    // only defaults nil to cthr->Obj) — re-enter through the nested seam so
    // the target's own scope runs the shift (the ObjectSetAction pattern).
    let active = active_object_id();
    if let Some(target) = target_object {
        if Some(target) != active {
            let forwarded = vec![
                Value::Nil,
                Value::Bool(shift_back),
                id_target.map(Value::C4Id).unwrap_or(Value::Nil),
                Value::Bool(do_calls),
            ];
            return match call_world_object_function(target, "ShiftContents", &forwarded) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    // Phase 1 (borrowed): resolve the frame-start contents view and pick
    // the new front — released before the DirectComContents calls re-enter.
    enum Picked {
        Done(Value),
        Shift {
            container: ObjectId,
            new_front: ObjectId,
            new_front_id: String,
        },
    }
    let picked = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Picked::Done(Value::Bool(false));
        };
        let Some(self_id) = context.object_context().map(|object| object.id()) else {
            return Picked::Done(Value::Bool(false));
        };
        let contents: Vec<(ObjectId, String)> = match context.get_world_object(self_id) {
            Some(container) => container
                .contents()
                .iter()
                .filter_map(|child_id| {
                    context
                        .get_world_object(*child_id)
                        .filter(|child| child.is_present())
                        .map(|child| (*child_id, child.definition_id().to_string()))
                })
                .collect(),
            None => return Picked::Done(Value::Bool(false)),
        };
        let Some((front_id, front_definition)) = contents.first().cloned() else {
            return Picked::Done(Value::Bool(false));
        };
        let new_front = if let Some(id_target) = id_target {
            // Check if the ID is present within the container
            // (C4Script.cpp:1790-1793).
            let Some((found, _)) = contents
                .iter()
                .find(|(_, definition)| *definition == id_target)
            else {
                return Picked::Done(Value::Bool(false));
            };
            // Desired object already at front? (DirectComContents :5759.)
            if *found == front_id {
                return Picked::Done(Value::Bool(true));
            }
            *found
        } else {
            // Walk next (or prev from the back with fShiftBack) for the
            // first DIFFERENT item (C4Object.cpp:5734-5750).
            let candidate = if shift_back {
                contents
                    .iter()
                    .skip(1)
                    .rev()
                    .find(|(_, definition)| *definition != front_definition)
            } else {
                contents
                    .iter()
                    .skip(1)
                    .find(|(_, definition)| *definition != front_definition)
            };
            match candidate {
                Some((id, _)) => *id,
                None => return Picked::Done(Value::Bool(false)),
            }
        };
        let new_front_id = contents
            .iter()
            .find(|(id, _)| *id == new_front)
            .map(|(_, definition)| definition.clone())
            .unwrap_or_default();
        Picked::Shift {
            container: self_id,
            new_front,
            new_front_id,
        }
    });
    let (container, new_front, new_front_id) = match picked {
        Picked::Done(value) => return Ok(value),
        Picked::Shift {
            container,
            new_front,
            new_front_id,
        } => (container, new_front, new_front_id),
    };
    // DirectComContents (C4Object.cpp:5760-5763): with fDoCalls the
    // container's ~ControlContents(idNewFront) runs first — a truthy
    // return takes over the selection (fail-safe exec, errors read false).
    if do_calls {
        let veto = match call_world_object_own_function(
            container,
            "ControlContents",
            &[Value::C4Id(new_front_id)],
        ) {
            Some(Ok(value)) => value.as_bool(),
            Some(Err(error)) => {
                tracing::warn!(
                    %error,
                    "script error in ControlContents; continuing like the C++ fail-safe exec"
                );
                false
            }
            None => false,
        };
        if veto {
            return Ok(Value::Bool(true));
        }
    }
    // The cyclic relink (C4ObjectList::ShiftContents, C4ObjectList.cpp:
    // 815-833) via ObjectUpdate.contents_front.
    let shifted = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|context| context.object_context_mut())
            .map(|object| object.shift_contents_front(new_front))
            .is_some()
    });
    if !shifted {
        return Ok(Value::Bool(false));
    }
    // ~Selection(container) on the new front; a falsy return plays the
    // Grab sound at the container (C4Object.cpp:5767).
    if do_calls {
        let selected = match call_world_object_own_function(
            new_front,
            "Selection",
            &[object_reference_value(container)],
        ) {
            Some(Ok(value)) => value.as_bool(),
            Some(Err(error)) => {
                tracing::warn!(
                    %error,
                    "script error in Selection; continuing like the C++ fail-safe exec"
                );
                false
            }
            None => false,
        };
        if !selected {
            HOST_CONTEXT.with(|cell| {
                if let Some(context) = cell.borrow_mut().as_mut() {
                    context
                        .audio_mut()
                        .play_sound("Grab", Some(container), 100, false, false, None);
                }
            });
        }
    }
    Ok(Value::Bool(true))
}

/// FnGetCommand (C4Script.cpp:918-945): walk the command stack to entry
/// iCommandNum and return the requested element. Element 0 (the C++
/// CommandName string) is served from the world context's frame-start
/// command-name snapshot; elements 1-5 (Target/Tx/Ty/Target2/Data) are
/// not yet threaded through the host contexts and yield nil with a
/// warning (documented gap).
fn get_command(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "GetCommand", "target")?;
            index += 1;
        }
    }
    let element = args
        .get(index)
        .map(|value| value_to_i32(value, "GetCommand", "element"))
        .transpose()?
        .unwrap_or(0);
    index += 1;
    let command_num = args
        .get(index)
        .map(|value| value_to_i32(value, "GetCommand", "command number"))
        .transpose()?
        .unwrap_or(0);

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let resolved = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(resolved) = resolved else {
            return Ok(Value::Nil);
        };
        let Some(world_object) = context.get_world_object(resolved) else {
            return Ok(Value::Nil);
        };
        // `while (Command && iCommandNum--)` (C4Script.cpp:924): a negative
        // count walks off the list end -> nil.
        if command_num < 0 {
            return Ok(Value::Nil);
        }
        let Some(name) = world_object.command_names.get(command_num as usize) else {
            return Ok(Value::Nil);
        };
        match element {
            0 => Ok(Value::String(name.clone())),
            1..=5 => {
                tracing::warn!(
                    element,
                    "GetCommand: Target/Tx/Ty/Target2/Data elements are not exposed yet; returning nil"
                );
                Ok(Value::Nil)
            }
            _ => Ok(Value::Nil),
        }
    })
}

fn get_action(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetAction", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetAction: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let action_name = object.effective_action_name();
                    let resolved = if action_name.is_empty() {
                        "Idle"
                    } else {
                        action_name
                    };
                    return Ok(Value::String(resolved.to_string()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let resolved = if other.action_name.is_empty() {
                    "Idle"
                } else {
                    other.action_name.as_str()
                };
                return Ok(Value::String(resolved.to_string()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let action_name = object.effective_action_name();
        let resolved = if action_name.is_empty() {
            "Idle"
        } else {
            action_name
        };
        Ok(Value::String(resolved.to_string()))
    })
}

fn get_act_time(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetActTime", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActTime: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let clamp_ticks = |ticks: u32| -> Value {
            let clamped = ticks.min(i32::MAX as u32) as i32;
            Value::Int(clamped)
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(clamp_ticks(object.effective_action_ticks()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(clamp_ticks(other.action_ticks()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(clamp_ticks(object.effective_action_ticks()))
    })
}

fn get_phase(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetPhase", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetPhase: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let object = if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    object
                } else if let Some(other) = context.get_world_object(target) {
                    return Ok(Value::Int(other.action_phase()));
                } else {
                    return Ok(Value::Nil);
                }
            } else if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.action_phase()));
            } else {
                return Ok(Value::Nil);
            }
        } else {
            match context.object_context() {
                Some(obj) => obj,
                None => return Ok(Value::Nil),
            }
        };

        Ok(Value::Int(object.action_phase()))
    })
}

fn set_phase(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetPhase expects at least 1 argument: phase",
        ));
    }

    let phase = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetPhase: expected int or nil for phase, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetPhase", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetPhase: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetPhase requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        // C4Object::SetPhase (C4Object.cpp:2205-2211): a no-op on idle
        // objects; the phase clamps to [0, Length] (BoundBy is INCLUSIVE
        // of Length — the wrap transition fires at the next ExecAction).
        let action_name = object.effective_action_name().to_string();
        if object.action_library.is_idle_action(&action_name) {
            return Ok(Value::Bool(false));
        }
        let length = object
            .action_library
            .specs()
            .get(&action_name)
            .and_then(|spec| spec.length)
            .map(|length| i32::try_from(length).unwrap_or(i32::MAX))
            .unwrap_or(1);
        object.set_action_phase(phase.clamp(0, length));
        Ok(Value::Bool(true))
    })
}

fn get_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetActionData", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActionData: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.effective_action_data()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.action_data()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(object.effective_action_data()))
    })
}

fn get_procedure(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetProcedure", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetProcedure: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let procedure_value = |name: Option<&str>| match name {
            Some(procedure) => Value::String(procedure.to_string()),
            None => Value::Nil,
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let procedure = object.effective_procedure_name();
                    return Ok(procedure_value(procedure));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(procedure_value(other.procedure_name()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let procedure = object.effective_procedure_name();
        Ok(procedure_value(procedure))
    })
}

fn get_action_target(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_index = 0;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                target_index = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    let mut object_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        object_id = parse_object_reference_argument(arg, "GetActionTarget", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActionTarget: additional arguments are not supported",
        ));
    }

    if target_index < 0 {
        return Ok(Value::Nil);
    }

    let slot = target_index as usize;
    if slot > 1 {
        return Ok(Value::Nil);
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = object_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let target_value = object.effective_action_target(slot);
                    return Ok(target_value.map_or(Value::Nil, object_reference_value));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let target_value = other.action_target(slot);
                return Ok(target_value.map_or(Value::Nil, object_reference_value));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let target_value = object.effective_action_target(slot);
        Ok(target_value.map_or(Value::Nil, object_reference_value))
    })
}

fn get_vertex_num(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(_) | Value::Nil => {
                target_id = parse_object_reference_argument(arg, "GetVertexNum", "object")?;
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetVertexNum: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        match resolve_vertices(context, target_id) {
            Some((_position, vertices)) => Ok(Value::Int(truncate_to_i32(vertices.len() as u64))),
            None => Ok(Value::Nil),
        }
    })
}

fn get_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetVertex: requires at least an index argument",
        ));
    }

    let index_value = value_to_i32(&args[0], "GetVertex", "index")?;
    let mut arg_index = 1;
    let mut attribute = 1;

    if let Some(arg) = args.get(arg_index) {
        match arg {
            Value::Int(value) => {
                attribute = *value;
                arg_index += 1;
            }
            Value::Nil => {
                arg_index += 1;
            }
            _ => {}
        }
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(arg_index) {
        target_id = parse_object_reference_argument(arg, "GetVertex", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetVertex: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (_position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        if vertices.is_empty() {
            return Ok(Value::Nil);
        }

        let limit = vertices.len() as i32 - 1;
        let mut clamped = index_value;
        if clamped < 0 {
            clamped = 0;
        } else if clamped > limit {
            clamped = limit;
        }
        let vertex = &vertices[clamped as usize];
        let result = match attribute {
            0 => vertex.x,
            1 => vertex.y,
            2 => truncate_to_i32(vertex.cnat as u64),
            3 => vertex.friction,
            _ => vertex.y,
        };
        Ok(Value::Int(result))
    })
}

/// FnInside (C4Script.cpp:3350-3353): value within [lo, hi] inclusive.
fn inside(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), "Inside", "value")?;
    let lo = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Inside", "lo")?;
    let hi = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Inside", "hi")?;
    Ok(Value::Bool(value >= lo && value <= hi))
}

/// FnGetVisibility (C4Script.cpp:3871-3877): pObj->Visibility. The
/// visibility register is draw-only and unmodeled (SetVisibility is an
/// acknowledged no-op) — reports the C4Object default 0 (VIS_All).
fn get_visibility(args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetVisibility", "obj"))
        .transpose()?;
    Ok(Value::Int(0))
}

/// FnSetCrewEnabled (C4Script.cpp:4814-4836): CrewDisabled = !enabled;
/// disabling also deselects (the cursor re-adjust runs on the engine
/// selection refresh).
/// FnGetColorDw (C4Script.cpp:3652-3656): the object's dword color;
/// nil without an object.
fn get_color_dw(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target =
        consume_optional_object_reference_argument(args, &mut index, "GetColorDw", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Nil);
        };
        // A same-call SetColorDw staged the value on the scope.
        if Some(target) == active {
            if let Some(color) = context
                .object_context()
                .and_then(|object| object.pending_update.color)
            {
                return Ok(Value::Int(color as i32));
            }
        }
        match context.get_world_object(target) {
            Some(object) => Ok(Value::Int(
                object
                    .full_state()
                    .map(|state| state.color as i32)
                    .unwrap_or(0),
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// FnGetChar (C4Script.cpp:4367-4376): the byte at the index, 0 past/// FnGetChar (C4Script.cpp:4367-4376): the byte at the index, 0 past
/// the end, nil without a string.
fn get_char(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(text)) = args.first() else {
        return Ok(Value::Nil);
    };
    let index = match args.get(1) {
        Some(Value::Int(value)) => *value,
        _ => 0,
    };
    if index < 0 {
        return Ok(Value::Int(0));
    }
    Ok(Value::Int(
        text.as_bytes()
            .get(index as usize)
            .map(|byte| i32::from(*byte))
            .unwrap_or(0),
    ))
}

/// FnGetCrewEnabled (C4Script.cpp:4813-4819): !CrewDisabled; nil/// FnGetCrewEnabled (C4Script.cpp:4813-4819): !CrewDisabled; nil
/// without an object.
fn get_crew_enabled(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target =
        consume_optional_object_reference_argument(args, &mut index, "GetCrewEnabled", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let active = context.object_context().map(|object| object.id());
        let target = target.or(active);
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        // A same-call SetCrewEnabled staged the flag on the scope.
        if Some(target) == active {
            if let Some(disabled) = context
                .object_context()
                .and_then(|object| object.pending_update.crew_disabled)
            {
                return Ok(Value::Bool(!disabled));
            }
        }
        match context.get_world_object(target) {
            Some(object) => Ok(Value::Bool(
                !object
                    .full_state()
                    .map(|state| state.crew_disabled)
                    .unwrap_or(false),
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// FnSetCursor (C4Script.cpp:2951-2958): set the player cursor (and
/// crew selection unless fNoSelectCrew).
fn set_cursor_host(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetCursor", "player")?;
    let object = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetCursor", "obj"))
        .transpose()?
        .flatten();
    let no_select_crew = matches!(args.get(4), Some(Value::Bool(true)) | Some(Value::Int(1)));
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if context.world.player(player_id).is_none() {
            return Ok(Value::Bool(false));
        }
        context.record_player_command(PlayerCommand::SetCursor {
            player_id,
            object,
            select_crew: !no_select_crew,
        });
        Ok(Value::Bool(true))
    })
}

fn set_crew_enabled(args: &[Value]) -> Result<Value, RuntimeError> {
    let enabled = matches!(args.first(), Some(Value::Bool(true)) | Some(Value::Int(1)));
    let target = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetCrewEnabled", "obj"))
        .transpose()?
        .flatten();
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetCrewEnabled",
                &[Value::Bool(enabled)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        object.pending_update.crew_disabled = Some(!enabled);
        Ok(Value::Bool(true))
    })
}

/// FnFinishCommand (C4Script.cpp:947-957): mark the iCommandNum-th
/// command of the target finished (success) or bump its failures.
fn finish_command(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "FinishCommand", "obj"))
        .transpose()?
        .flatten();
    let success = matches!(args.get(1), Some(Value::Bool(true)) | Some(Value::Int(1)));
    let index = parse_optional_i32(args.get(2), "FinishCommand", "command")?.unwrap_or(0);
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "FinishCommand",
                &[Value::Int(0), Value::Bool(success), Value::Int(index)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        object
            .command_operations
            .push(CommandOperation::Finish { index, success });
        Ok(Value::Bool(true))
    })
}

/// FnFling (C4Script.cpp:347-356) -> C4Object::Fling
/// (C4Object.cpp:1612-1624): optional half-speed add, then the Tumble
/// action if the ActMap has one, else the Jump action (after the
/// OnActionJump script hook), else raw velocity; the action-callback
/// bottom attach is cleared either way.
fn fling(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Fling", "obj"))
        .transpose()?
        .flatten();
    let xdir = parse_optional_i32(args.get(1), "Fling", "xdir")?.unwrap_or(0);
    let ydir = parse_optional_i32(args.get(2), "Fling", "ydir")?.unwrap_or(0);
    let prec = parse_optional_i32(args.get(3), "Fling", "precision")?
        .filter(|&prec| prec != 0)
        .unwrap_or(1);
    let add_speed = matches!(args.get(4), Some(Value::Bool(true)) | Some(Value::Int(1)));
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            let mut forwarded = vec![
                Value::Nil,
                Value::Int(xdir),
                Value::Int(ydir),
                Value::Int(prec),
                Value::Bool(add_speed),
            ];
            forwarded[0] = Value::Int(0);
            return match call_world_object_function(target, "Fling", &forwarded) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        let mut txdir = itofix_prec(xdir, prec);
        let mut tydir = itofix_prec(ydir, prec);
        if add_speed {
            let velocity = object.fixed_velocity();
            txdir += velocity.x / 2;
            tydir += velocity.y / 2;
        }
        let tumble = object.action_library.contains("Tumble");
        let jump = object.action_library.contains("Jump");
        if tumble {
            object.pending_update.direction = Some(if txdir < C4Fixed::ZERO {
                Direction::Left
            } else {
                Direction::Right
            });
            object.pending_update.action = Some(
                ActionUpdate::default()
                    .with_name("Tumble")
                    .with_phase(0)
                    .with_ticks(0)
                    .with_force(true),
            );
        } else if jump {
            object.pending_update.action = Some(
                ActionUpdate::default()
                    .with_name("Jump")
                    .with_phase(0)
                    .with_ticks(0)
                    .with_force(true),
            );
        }
        object.set_fixed_velocity(FixedVec2 { x: txdir, y: tydir });
        Ok(Value::Bool(true))
    })
}

/// FnJump (C4Script.cpp:358-363): ObjectComJump — routed as a
/// front-pushed Jump command on the target (the executor runs the
/// walking gate + jump kinematics).
fn jump(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Jump", "obj"))
        .transpose()?
        .flatten();
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(target, "Jump", &[]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    // FnJump → ObjectComJump runs SYNCHRONOUSLY (C4Script.cpp:358-363,
    // C4ObjectCom.cpp:280-312): the snake's Activity jump takes effect
    // THIS frame, before its movement — a queued command would lag one
    // frame. Gates: only while the WALK procedure runs.
    let launch = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return None;
        };
        let object = context.object_context_mut()?;
        let action_name = object.effective_action_name().to_string();
        if !matches!(
            object.action_library.procedure_for_action(&action_name),
            crate::action::ActionProcedure::Walk
        ) {
            return None;
        }
        let walk = object.get_physical("Walk", PHYS_CURRENT).unwrap_or(0);
        let jump_physical = object.get_physical("Jump", PHYS_CURRENT).unwrap_or(0);
        let con_scale = itofix_prec(object.construction(), crate::FULL_CON);
        let physical_walk = crate::math::val_by_physical(280, walk) * con_scale;
        let physical_jump = crate::math::val_by_physical(1000, jump_physical) * con_scale;
        let txdir = match object.command_direction() {
            CommandDirection::Left | CommandDirection::UpLeft => -physical_walk,
            CommandDirection::Right | CommandDirection::UpRight => physical_walk,
            _ => match object.direction() {
                Direction::Left => -physical_walk,
                Direction::Right => physical_walk,
            },
        };
        // The dive branch (SimFlightHitsLiquid → ObjectActionDive,
        // C4ObjectCom.cpp:301-305) is a documented PORT_STATUS gap.
        Some((txdir, -physical_jump))
    });
    let Some((txdir, tydir)) = launch else {
        return Ok(Value::Bool(false));
    };
    // ObjectActionJump (C4ObjectCom.cpp:48-61): SetActionByName("Jump")
    // with its Abort/Start calls, then the launch velocities. A failed
    // SetAction (no Jump in the ActMap) returns false without touching
    // the dirs.
    let set = set_action(&[Value::String("Jump".to_string())])?;
    if !matches!(set, Value::Bool(true)) {
        return Ok(Value::Bool(false));
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(true));
        };
        if let Some(object) = context.object_context_mut() {
            object.set_fixed_velocity(FixedVec2::new(txdir, tydir));
        }
        Ok(Value::Bool(true))
    })
}

/// FnEnergyCheck (C4Script.cpp:1845-1856): true when the
/// StructuresNeedEnergy rule is off, the object has enough energy, or
/// the def is not a power consumer. (The NeedEnergy HUD flag is
/// unmodeled.)
fn energy_check(args: &[Value]) -> Result<Value, RuntimeError> {
    let energy = value_to_i32(args.first().unwrap_or(&Value::Nil), "EnergyCheck", "energy")?;
    let target = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "EnergyCheck", "obj"))
        .transpose()?
        .flatten();
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if !context.world.structures_need_energy() {
            return Ok(Value::Bool(true));
        }
        let target = target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let Some(object) = context.get_world_object(target) else {
            return Ok(Value::Nil);
        };
        if object.energy >= energy {
            return Ok(Value::Bool(true));
        }
        let is_consumer = context
            .world
            .definition_metadata(&object.definition_id)
            .map(|metadata| metadata.line_connect & 32 != 0)
            .unwrap_or(false);
        Ok(Value::Bool(!is_consumer))
    })
}

/// FnStuck (C4Script.cpp:1858-1862): Shape.CheckContact(x, y) — is any
/// shape vertex inside solid at the current position (C4Shape.cpp
/// CheckContact probes GBackSolid per vertex).
fn stuck(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Stuck", "obj"))
        .transpose()?
        .flatten();
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some((position, vertices)) = resolve_vertices(context, target_id) else {
            return Ok(Value::Nil);
        };
        let Some(landscape) = context.landscape_ref() else {
            return Ok(Value::Bool(false));
        };
        let stuck = vertices
            .iter()
            .any(|vertex| landscape.is_solid_at(position.x + vertex.x, position.y + vertex.y));
        Ok(Value::Bool(stuck))
    })
}

fn get_vertex_contact(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetVertexContact: requires a vertex index argument",
        ));
    }

    let vertex_index = value_to_i32(&args[0], "GetVertexContact", "index")?;
    let mut arg_index = 1;
    let mut mask: u32 = 0;

    if let Some(arg) = args.get(arg_index) {
        match arg {
            Value::Int(value) => {
                if *value > 0 {
                    mask = *value as u32;
                }
                arg_index += 1;
            }
            Value::Nil => {
                arg_index += 1;
            }
            _ => {}
        }
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(arg_index) {
        target_id = parse_object_reference_argument(arg, "GetVertexContact", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetVertexContact: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let landscape = context.landscape_ref();
        let contact =
            compute_vertex_contact(landscape, position, &vertices[vertex_index as usize], mask);
        Ok(Value::Int(contact as i32))
    })
}

fn get_contact(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnGetContact(pObj, iVertex, dwCheck) — C4Script.cpp:5611-5626:
    // the OBJECT comes first; iVertex -1 ORs all vertex contacts. A
    // non-object in the pObj slot coerces to nil (self) like C4Value.
    let target_id = match args.first() {
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            parse_object_reference_argument(value, "GetContact", "obj")?
        }
        _ => None,
    };
    let vertex_index = match args.get(1) {
        None | Some(Value::Nil) => -1,
        Some(value) => value_to_i32(value, "GetContact", "vertex")?,
    };
    let mask = match args.get(2) {
        None | Some(Value::Nil) => 0u32,
        Some(value) => {
            let raw = value_to_i32(value, "GetContact", "mask")?;
            if raw > 0 {
                raw as u32
            } else {
                0
            }
        }
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        let landscape = context.landscape_ref();

        if vertex_index == -1 {
            if vertices.is_empty() {
                return Ok(Value::Int(0));
            }
            let mut result = 0u32;
            for vertex in vertices {
                result |= compute_vertex_contact(landscape, position, vertex, mask);
            }
            return Ok(Value::Int(result as i32));
        }

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let contact =
            compute_vertex_contact(landscape, position, &vertices[vertex_index as usize], mask);
        Ok(Value::Int(contact as i32))
    })
}

fn path_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 4 {
        return Err(RuntimeError::new(
            "PathFree expects 4 arguments: x1, y1, x2, y2",
        ));
    }

    let x1 = value_to_i32(&args[0], "PathFree", "x1")?;
    let y1 = value_to_i32(&args[1], "PathFree", "y1")?;
    let x2 = value_to_i32(&args[2], "PathFree", "x2")?;
    let y2 = value_to_i32(&args[3], "PathFree", "y2")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Bool(true)),
        };

        let Some(landscape) = context.landscape_ref() else {
            return Ok(Value::Bool(true));
        };

        // FnPathFree → ::PathFree = the ForLine per-pixel Bresenham with
        // GBackSolid blocking (C4Landscape.cpp:1683-1738, 2052-2055) —
        // its exact stepping decides script branches (the Bison's
        // EnemyNearby flee gate). GBackSolid sees C4SolidMask's baked
        // MCVehic pixels — the plane carries them (put_solid_mask).
        let clear = match context.world.materials() {
            Some(materials) => {
                crate::path_free_exact(landscape, materials, &[], x1, y1, x2, y2)
            }
            None => landscape.path_is_clear(Vector2::new(x1, y1), Vector2::new(x2, y2)),
        };
        Ok(Value::Bool(clear))
    })
}

fn get_path(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 4 {
        return Err(RuntimeError::new(
            "GetPath expects 4 arguments: from_x, from_y, to_x, to_y",
        ));
    }

    let from_x = value_to_i32(&args[0], "GetPath", "from_x")?;
    let from_y = value_to_i32(&args[1], "GetPath", "from_y")?;
    let to_x = value_to_i32(&args[2], "GetPath", "to_x")?;
    let to_y = value_to_i32(&args[3], "GetPath", "to_y")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let landscape = match context.landscape_ref() {
            Some(landscape) => landscape,
            None => return Ok(Value::Nil),
        };
        let mut finder = PathFinder::new(landscape, context.world.transfer_zones());
        let path = match finder.find(Vector2::new(from_x, from_y), Vector2::new(to_x, to_y)) {
            Some(path) => path,
            None => return Ok(Value::Nil),
        };
        let mut result = HashMap::new();
        result.insert("Length".into(), Value::Int(path.length));
        let mut waypoints = Vec::with_capacity(path.waypoints.len());
        for waypoint in path.waypoints {
            let mut map = HashMap::new();
            map.insert("X".into(), Value::Int(waypoint.x));
            map.insert("Y".into(), Value::Int(waypoint.y));
            if let Some(target) = waypoint.transfer_target {
                map.insert("TransferTarget".into(), object_reference_value(target));
            }
            waypoints.push(Value::Proplist(map));
        }
        result.insert("Waypoints".into(), Value::Array(waypoints));
        Ok(Value::Proplist(result))
    })
}

fn set_transfer_zone(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 4 || args.len() > 5 {
        return Err(RuntimeError::new(
            "SetTransferZone expects 4 or 5 arguments: x, y, width, height, [object]",
        ));
    }

    let x = value_to_i32(&args[0], "SetTransferZone", "x")?;
    let y = value_to_i32(&args[1], "SetTransferZone", "y")?;
    let width = value_to_i32(&args[2], "SetTransferZone", "width")?;
    let height = value_to_i32(&args[3], "SetTransferZone", "height")?;
    let explicit_object = if args.len() == 5 {
        Some(parse_object_reference_argument(
            &args[4],
            "SetTransferZone",
            "object",
        )?)
    } else {
        None
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetTransferZone requires an active engine context")
        })?;

        let owner = match explicit_object.flatten() {
            Some(id) => id,
            None => context
                .object_context()
                .map(|ctx| ctx.id())
                .ok_or_else(|| {
                    RuntimeError::new(
                        "SetTransferZone requires an object argument or active object context",
                    )
                })?,
        };

        let world_object = context.get_world_object(owner).ok_or_else(|| {
            RuntimeError::new(format!(
                "SetTransferZone: object {} not found in current engine context",
                owner
            ))
        })?;

        if width <= 0 || height <= 0 {
            context.register_transfer_zone_command(TransferZoneCommand::clear(owner));
            return Ok(Value::Bool(true));
        }

        let abs_x = world_object.position.x.saturating_add(x);
        let abs_y = world_object.position.y.saturating_add(y);
        let rect = TransferZoneRect {
            x: abs_x,
            y: abs_y,
            width,
            height,
        };
        context.register_transfer_zone_command(TransferZoneCommand::set(owner, rect));
        Ok(Value::Bool(true))
    })
}

#[derive(Clone, Copy)]
enum LandscapeQuery {
    Solid,
    SemiSolid,
    Liquid,
    Sky,
}

fn g_back_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSolid", LandscapeQuery::Solid)
}

fn g_back_semi_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSemiSolid", LandscapeQuery::SemiSolid)
}

fn g_back_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackLiquid", LandscapeQuery::Liquid)
}

fn g_back_sky(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSky", LandscapeQuery::Sky)
}

fn g_back_common(
    args: &[Value],
    function: &str,
    query: LandscapeQuery,
) -> Result<Value, RuntimeError> {
    // Unfilled parameter slots are nil -> 0 (C4Aul.h:104-121,
    // C4AulExec.cpp:1364-1396): GBackSolid() queries the object's position.
    let local_x = value_to_i32(args.first().unwrap_or(&Value::Nil), function, "x")?;
    let local_y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "y")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Bool(fallback_without_context(query))),
        };

        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }

        let landscape = context.landscape_ref();
        let result = evaluate_landscape_query(landscape, query, global_x, global_y);
        Ok(Value::Bool(result))
    })
}

fn evaluate_landscape_query(
    landscape: Option<&Landscape>,
    query: LandscapeQuery,
    x: i32,
    y: i32,
) -> bool {
    match landscape {
        Some(landscape) => match query {
            LandscapeQuery::Solid => landscape.is_solid_at(x, y),
            // GBackSemiSolid = density >= C4M_SemiSolid(25), which liquids
            // satisfy (C4Wrappers.h:73-76, C4Material.h:202).
            LandscapeQuery::SemiSolid => landscape.is_semi_solid_at(x, y),
            LandscapeQuery::Liquid => landscape.is_liquid_at(x, y),
            LandscapeQuery::Sky => !landscape.is_solid_at(x, y),
        },
        None => fallback_without_context(query),
    }
}

fn fallback_without_context(query: LandscapeQuery) -> bool {
    match query {
        LandscapeQuery::Sky => true,
        LandscapeQuery::Solid | LandscapeQuery::SemiSolid | LandscapeQuery::Liquid => false,
    }
}

fn get_material(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ pads missing script args with zero (FnGetMaterial(x, y),
    // C4Script.cpp:2222-2226): GetMaterial() probes the object center.
    let local_x = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetMaterial", "x")?,
    };
    let local_y = match args.get(1) {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetMaterial", "y")?,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(MATERIAL_NONE)),
        };

        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }

        let material = context
            .landscape_ref()
            .and_then(|landscape| landscape.material_at(global_x, global_y));
        let result = material
            .map(|material_id| material_id.index() as i32)
            .unwrap_or(MATERIAL_NONE);
        Ok(Value::Int(result))
    })
}

fn blast_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "BlastFree expects at least 3 arguments: x, y, level",
        ));
    }
    if args.len() > 4 {
        return Err(RuntimeError::new(
            "BlastFree expects at most 4 arguments: x, y, level, caused_by",
        ));
    }

    let mut x = value_to_i32(&args[0], "BlastFree", "x")?;
    let mut y = value_to_i32(&args[1], "BlastFree", "y")?;
    let level = value_to_i32(&args[2], "BlastFree", "level")?;
    if level <= 0 {
        return Ok(Value::Bool(false));
    }

    let mut caused_by_plus_one = 0;
    if let Some(arg) = args.get(3) {
        caused_by_plus_one = match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "BlastFree: expected int or nil for caused by, got {}",
                    other.type_name()
                )))
            }
        };
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("BlastFree requires an active engine context"))?;

        let mut controller = if caused_by_plus_one > 0 {
            Some(caused_by_plus_one - 1)
        } else {
            None
        };

        if caused_by_plus_one <= 0 {
            if let Some(object) = context.object_context() {
                let position = object.effective_position();
                x = x.saturating_add(position.x);
                y = y.saturating_add(position.y);
                if controller.is_none() {
                    controller = Some(object.owner());
                }
            }
        }

        context.register_landscape_operation(LandscapeOperation::BlastCircle {
            center: Vector2::new(x, y),
            radius: level,
            controller,
        });
        Ok(Value::Bool(true))
    })
}

fn shake_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "ShakeFree expects at least 3 arguments: x, y, radius",
        ));
    }
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "ShakeFree expects exactly 3 arguments: x, y, radius",
        ));
    }

    let x = value_to_i32(&args[0], "ShakeFree", "x")?;
    let y = value_to_i32(&args[1], "ShakeFree", "y")?;
    let radius = value_to_i32(&args[2], "ShakeFree", "radius")?;
    if radius <= 0 {
        return Ok(Value::Bool(false));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("ShakeFree requires an active engine context"))?;
        context.register_landscape_operation(LandscapeOperation::ShakeCircle {
            center: Vector2::new(x, y),
            radius,
        });
        Ok(Value::Bool(true))
    })
}

fn dig_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "DigFree expects at least 3 arguments: x, y, radius",
        ));
    }

    let x = value_to_i32(&args[0], "DigFree", "x")?;
    let y = value_to_i32(&args[1], "DigFree", "y")?;
    let radius = value_to_i32(&args[2], "DigFree", "radius")?;
    if radius < 0 {
        return Ok(Value::Bool(false));
    }

    let requested = if let Some(arg) = args.get(3) {
        value_to_bool(arg, "DigFree", "requested")?
    } else {
        false
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DigFree requires an active engine context"))?;
        let by_object = context.object_context().map(|object| object.id());
        context.register_landscape_operation(LandscapeOperation::DigCircle {
            center: Vector2::new(x, y),
            radius,
            requested,
            by_object,
        });
        Ok(Value::Bool(true))
    })
}

/// FnFreeRect (C4Script.cpp:3125-3131): clears the landscape rect in
/// GLOBAL coordinates (no caller offset, unlike DigFree*) without
/// producing dug-out material. The density-filtered form
/// (iFreeDensity -> ClearRectDensity) clears everything in the column
/// model (PORT_STATUS).
/// FnScriptGo (C4Script.cpp:2782-2786): switches the scenario script
/// counter gate (Game.Script.Go) that drives the timed Script%d
/// sections (C4GameScriptHost::Execute, C4ScriptHost.cpp:222-232).
fn script_go(args: &[Value]) -> Result<Value, RuntimeError> {
    let go = match args.first() {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Int(value)) => *value != 0,
        _ => false,
    };
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(true));
        };
        context.script_go_request = Some(go);
        Ok(Value::Bool(true))
    })
}

fn free_rect(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "FreeRect", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "FreeRect", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "FreeRect", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "FreeRect", "hgt")?;
    if width <= 0 || height <= 0 {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        context.register_landscape_operation(LandscapeOperation::ClearRect {
            origin: Vector2::new(x, y),
            width,
            height,
        });
        Ok(Value::Nil)
    })
}

fn dig_free_rect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 4 {
        return Err(RuntimeError::new(
            "DigFreeRect expects at least 4 arguments: x, y, width, height",
        ));
    }

    let x = value_to_i32(&args[0], "DigFreeRect", "x")?;
    let y = value_to_i32(&args[1], "DigFreeRect", "y")?;
    let width = value_to_i32(&args[2], "DigFreeRect", "width")?;
    let height = value_to_i32(&args[3], "DigFreeRect", "height")?;
    if width <= 0 || height <= 0 {
        return Ok(Value::Bool(false));
    }

    let requested = if let Some(arg) = args.get(4) {
        value_to_bool(arg, "DigFreeRect", "requested")?
    } else {
        false
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DigFreeRect requires an active engine context"))?;
        let by_object = context.object_context().map(|object| object.id());
        context.register_landscape_operation(LandscapeOperation::DigRect {
            origin: Vector2::new(x, y),
            width,
            height,
            requested,
            by_object,
        });
        Ok(Value::Bool(true))
    })
}

// ── C4FindObject / C4SortObject condition trees (C4FindObject.{h,cpp}) ──────

/// C4FO_* constants (C4FindObject.h:27-50) as a parsed condition tree.
/// Known divergences: `Layer` is unmodeled on host objects (never
/// matches); shape tests use the vertices bounding box.
#[derive(Debug, Clone)]
enum FindCondition {
    Not(Box<FindCondition>),
    And(Vec<FindCondition>),
    Or(Vec<FindCondition>),
    Exclude(Option<ObjectId>),
    Id(String),
    InRect(DefinitionRect),
    AtPoint(i32, i32),
    AtRect(DefinitionRect),
    OnLine(i32, i32, i32, i32),
    Distance {
        x: i32,
        y: i32,
        r2: i64,
    },
    Ocf(u32),
    Category(i32),
    Action(String),
    ActionTarget {
        target: Option<ObjectId>,
        index: usize,
    },
    Container(Option<ObjectId>),
    AnyContainer,
    Owner(i32),
    Controller(i32),
    /// C4FindObjectFunc (C4FindObject.cpp:124-136): calls `name` on each
    /// candidate via the nested-call seam.
    Func {
        name: String,
        pars: Vec<Value>,
    },
    Layer,
}

/// C4SO_* constants (C4FindObject.h:53-62) as a parsed sort tree.
#[derive(Debug, Clone)]
enum SortCriterion {
    Reverse(Box<SortCriterion>),
    Multiple(Vec<SortCriterion>),
    Distance {
        x: i32,
        y: i32,
    },
    Random,
    Speed,
    Mass,
    Value,
    /// C4SortObjectFunc (C4FindObject.h:521-533). Cached evaluation lands
    /// with the Sort_Func slice; until then all values compare equal.
    Func {
        name: String,
        pars: Vec<Value>,
    },
}

enum ParsedCriterion {
    Condition(FindCondition),
    Sort(SortCriterion),
    None,
}

fn value_as_object_id(value: &Value) -> Option<ObjectId> {
    match value {
        Value::Object(id) => Some(ObjectId::new(*id)),
        _ => object_id_from_value(value),
    }
}

fn value_as_i32(value: &Value) -> i32 {
    match value {
        Value::Int(value) => *value,
        Value::Bool(value) => i32::from(*value),
        _ => 0,
    }
}

impl FindCondition {
    /// `C4FindObject::CreateByValue` (C4FindObject.cpp:37-162): arrays whose
    /// first element is in C4SO_First..=C4SO_Last parse as sort criteria
    /// instead.
    fn parse(value: &Value) -> ParsedCriterion {
        // Must be an array (C4FindObject.cpp:40-41)
        let Value::Array(data) = value else {
            return ParsedCriterion::None;
        };
        let kind = data.first().map(value_as_i32).unwrap_or(0);
        if (100..=200).contains(&kind) {
            return SortCriterion::parse_typed(kind, data)
                .map(ParsedCriterion::Sort)
                .unwrap_or(ParsedCriterion::None);
        }
        let arg_i32 = |index: usize| data.get(index).map(value_as_i32).unwrap_or(0);
        let condition = match kind {
            // C4FO_Not
            1 => match data.get(1).map(Self::parse) {
                Some(ParsedCriterion::Condition(child)) => FindCondition::Not(Box::new(child)),
                _ => return ParsedCriterion::None,
            },
            // C4FO_And / C4FO_Or: trivial single-condition unwrap, dropped
            // null children (C4FindObject.cpp:67-87)
            2 | 3 => {
                let children: Vec<FindCondition> = data[1..]
                    .iter()
                    .filter_map(|entry| match Self::parse(entry) {
                        ParsedCriterion::Condition(child) => Some(child),
                        _ => None,
                    })
                    .collect();
                if data.len() == 2 {
                    match children.into_iter().next() {
                        Some(child) => child,
                        None => return ParsedCriterion::None,
                    }
                } else if kind == 2 {
                    FindCondition::And(children)
                } else {
                    FindCondition::Or(children)
                }
            }
            // C4FO_Exclude
            5 => FindCondition::Exclude(data.get(1).and_then(value_as_object_id)),
            // C4FO_InRect
            10 => FindCondition::InRect(DefinitionRect::new(
                arg_i32(1),
                arg_i32(2),
                arg_i32(3),
                arg_i32(4),
            )),
            // C4FO_AtPoint
            11 => FindCondition::AtPoint(arg_i32(1), arg_i32(2)),
            // C4FO_AtRect
            12 => FindCondition::AtRect(DefinitionRect::new(
                arg_i32(1),
                arg_i32(2),
                arg_i32(3),
                arg_i32(4),
            )),
            // C4FO_OnLine
            13 => FindCondition::OnLine(arg_i32(1), arg_i32(2), arg_i32(3), arg_i32(4)),
            // C4FO_Distance
            14 => {
                let r = i64::from(arg_i32(3));
                FindCondition::Distance {
                    x: arg_i32(1),
                    y: arg_i32(2),
                    r2: r * r,
                }
            }
            // C4FO_ID
            20 => match data.get(1) {
                Some(Value::C4Id(id)) => FindCondition::Id(id.clone()),
                Some(Value::String(id)) => FindCondition::Id(id.clone()),
                _ => return ParsedCriterion::None,
            },
            // C4FO_OCF
            21 => FindCondition::Ocf(arg_i32(1) as u32),
            // C4FO_Category
            22 => FindCondition::Category(arg_i32(1)),
            // C4FO_Action
            30 => match data.get(1) {
                Some(Value::String(name)) => FindCondition::Action(name.clone()),
                _ => return ParsedCriterion::None,
            },
            // C4FO_ActionTarget (index clamped to 0..=1, C4FindObject.cpp:138-144)
            31 => FindCondition::ActionTarget {
                target: data.get(1).and_then(value_as_object_id),
                index: arg_i32(2).clamp(0, 1) as usize,
            },
            // C4FO_Container
            40 => FindCondition::Container(data.get(1).and_then(value_as_object_id)),
            // C4FO_AnyContainer
            41 => FindCondition::AnyContainer,
            // C4FO_Owner
            50 => FindCondition::Owner(arg_i32(1)),
            // C4FO_Controller
            51 => FindCondition::Controller(arg_i32(1)),
            // C4FO_Func: Data[1] must convert to a string, else the whole
            // criterion is dropped (C4FindObject.cpp:127-128); Data[2] →
            // par 0, capped at 10 pars (SetPar, C4FindObject.cpp:645-651)
            60 => match data.get(1) {
                Some(Value::String(name)) => FindCondition::Func {
                    name: name.clone(),
                    pars: data.iter().skip(2).take(10).cloned().collect(),
                },
                _ => return ParsedCriterion::None,
            },
            // C4FO_Layer
            70 => FindCondition::Layer,
            _ => return ParsedCriterion::None,
        };
        ParsedCriterion::Condition(condition)
    }

    /// Per-condition Check (C4FindObject.cpp:390-679). Fallible because a
    /// `Func` callback error passes through (`fPassErrors=true`,
    /// C4FindObject.cpp:661); And/Or evaluate children in array order with
    /// short-circuit, so Func side effects land in C++ order.
    fn check(
        &self,
        world: &impl WorldAccessor,
        object: &HostWorldObject,
    ) -> Result<bool, RuntimeError> {
        Ok(match self {
            FindCondition::Not(child) => !child.check(world, object)?,
            FindCondition::And(children) => {
                for child in children {
                    if !child.check(world, object)? {
                        return Ok(false);
                    }
                }
                true
            }
            FindCondition::Or(children) => {
                for child in children {
                    if child.check(world, object)? {
                        return Ok(true);
                    }
                }
                false
            }
            FindCondition::Exclude(excluded) => Some(object.id) != *excluded,
            FindCondition::Id(id) => object.definition_id() == id,
            FindCondition::InRect(rect) => {
                // C4FindObjectInRect::Check is a plain point-in-rect on the
                // object CENTER (C4FindObject.cpp) — the old
                // contains_offset(pos - rect.xy) clause double-subtracted
                // the origin and matched far-away objects whose offset
                // happened to land back inside (the 597 NoDmg class).
                let position = object.position();
                position.x >= rect.x
                    && position.x < rect.x + rect.width
                    && position.y >= rect.y
                    && position.y < rect.y + rect.height
            }
            FindCondition::AtPoint(x, y) => {
                let metadata = world.definition_metadata(object.definition_id());
                compute_object_bounds(object, metadata.as_ref())
                    .map(|(left, top, right, bottom)| {
                        *x >= left && *x < right && *y >= top && *y < bottom
                    })
                    .unwrap_or(false)
            }
            FindCondition::AtRect(rect) => {
                let metadata = world.definition_metadata(object.definition_id());
                compute_object_bounds(object, metadata.as_ref())
                    .map(|(left, top, right, bottom)| {
                        rect.x < right
                            && rect.x + rect.width > left
                            && rect.y < bottom
                            && rect.y + rect.height > top
                    })
                    .unwrap_or(false)
            }
            FindCondition::OnLine(x1, y1, x2, y2) => {
                let metadata = world.definition_metadata(object.definition_id());
                compute_object_bounds(object, metadata.as_ref())
                    .map(|bounds| segment_intersects_bounds(*x1, *y1, *x2, *y2, bounds))
                    .unwrap_or(false)
            }
            FindCondition::Distance { x, y, r2 } => {
                let position = object.position();
                let dx = i64::from(position.x - x);
                let dy = i64::from(position.y - y);
                dx * dx + dy * dy <= *r2
            }
            FindCondition::Ocf(mask) => object.ocf() & mask != 0,
            FindCondition::Category(category) => object.category() & category != 0,
            FindCondition::Action(name) => object.action_name() == name,
            FindCondition::ActionTarget { target, index } => {
                object.action_target(*index) == *target
            }
            FindCondition::Container(container) => object.container() == *container,
            FindCondition::AnyContainer => object.container().is_some(),
            FindCondition::Owner(owner) => object.owner() == *owner,
            // C4FindObjectController::Check (C4FindObject.cpp:628-631).
            FindCondition::Controller(controller) => object.controller() == *controller,
            // C4FindObjectFunc::Check (C4FindObject.cpp:653-662): no
            // overload visible to the object's def → silently false; the
            // result converts with raw C4Value truthiness, not getBool.
            FindCondition::Func { name, pars } => {
                match call_world_object_function(object.id, name, pars) {
                    None => false,
                    Some(result) => value_raw_truthy(&result?),
                }
            }
            FindCondition::Layer => false,
        })
    }

    /// IsImpossible/IsEnsured pruning (C4FindObject.cpp:453-590). `Func` is
    /// impossible only when the name is unknown to every script
    /// (GetFirstFunc miss at construction, C4FindObject.cpp:640-643,
    /// 664-667); Not swaps the two (C4FindObject.h:116-117).
    fn is_impossible(&self, world: &impl WorldAccessor) -> bool {
        match self {
            FindCondition::Not(child) => child.is_ensured(world),
            FindCondition::And(children) => children.iter().any(|child| child.is_impossible(world)),
            FindCondition::Or(children) => {
                !children.iter().any(|child| !child.is_impossible(world))
            }
            FindCondition::InRect(rect) => rect.width == 0 || rect.height == 0,
            FindCondition::Ocf(mask) => *mask == 0,
            FindCondition::Func { name, .. } => !world.script_function_known(name),
            _ => false,
        }
    }

    fn is_ensured(&self, world: &impl WorldAccessor) -> bool {
        match self {
            FindCondition::Not(child) => child.is_impossible(world),
            FindCondition::Category(category) => *category == 0,
            _ => false,
        }
    }

    /// Whether any node needs the nested-call seam (drives the borrow-free
    /// snapshot-view evaluation path in the drivers).
    fn uses_func(&self) -> bool {
        match self {
            FindCondition::Not(child) => child.uses_func(),
            FindCondition::And(children) | FindCondition::Or(children) => {
                children.iter().any(FindCondition::uses_func)
            }
            FindCondition::Func { .. } => true,
            _ => false,
        }
    }
}

/// Axis-aligned segment/box intersection for C4FO_OnLine (the C++ uses
/// `Shape.IntersectsLine`; the host shape is its vertices bounding box).
fn segment_intersects_bounds(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    bounds: (i32, i32, i32, i32),
) -> bool {
    let (left, top, right, bottom) = bounds;
    let inside = |x: i32, y: i32| x >= left && x < right && y >= top && y < bottom;
    if inside(x1, y1) || inside(x2, y2) {
        return true;
    }
    // sample the segment at integer steps (sufficient for the pixel grid)
    let steps = (x2 - x1).abs().max((y2 - y1).abs());
    for step in 0..=steps {
        let x = x1 + (x2 - x1) * step / steps.max(1);
        let y = y1 + (y2 - y1) * step / steps.max(1);
        if inside(x, y) {
            return true;
        }
    }
    false
}

impl SortCriterion {
    /// `C4SortObject::CreateByValue` (C4FindObject.cpp:683-758).
    fn parse_typed(kind: i32, data: &[Value]) -> Option<SortCriterion> {
        let arg_i32 = |index: usize| data.get(index).map(value_as_i32).unwrap_or(0);
        Some(match kind {
            // C4SO_Reverse
            101 => SortCriterion::Reverse(Box::new(data.get(1).and_then(Self::parse)?)),
            // C4SO_Multiple (trivial single unwrap, C4FindObject.cpp:705-726)
            102 => {
                let children: Vec<SortCriterion> =
                    data[1..].iter().filter_map(Self::parse).collect();
                if data.len() == 2 {
                    children.into_iter().next()?
                } else {
                    SortCriterion::Multiple(children)
                }
            }
            // C4SO_Distance
            110 => SortCriterion::Distance {
                x: arg_i32(1),
                y: arg_i32(2),
            },
            // C4SO_Random
            120 => SortCriterion::Random,
            // C4SO_Speed
            130 => SortCriterion::Speed,
            // C4SO_Mass
            140 => SortCriterion::Mass,
            // C4SO_Value
            150 => SortCriterion::Value,
            // C4SO_Func: string name required, else nullptr
            // (C4FindObject.cpp:743-755); pars capped at 10
            160 => match data.get(1) {
                Some(Value::String(name)) => SortCriterion::Func {
                    name: name.clone(),
                    pars: data.iter().skip(2).take(10).cloned().collect(),
                },
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Whether any node needs the nested-call seam.
    fn uses_func(&self) -> bool {
        match self {
            SortCriterion::Reverse(child) => child.uses_func(),
            SortCriterion::Multiple(children) => children.iter().any(SortCriterion::uses_func),
            SortCriterion::Func { .. } => true,
            _ => false,
        }
    }

    fn parse(value: &Value) -> Option<SortCriterion> {
        let Value::Array(data) = value else {
            return None;
        };
        Self::parse_typed(data.first().map(value_as_i32).unwrap_or(0), data)
    }

    /// `CompareGetValue` (C4FindObject.cpp:908-956). `Random` draws the
    /// synced `Random(1 << 16)` — exactly once per object, in collection
    /// order, via the cache (C4SortObjectByValue::PrepareCache). `Func`
    /// runs the nested call: no overload → 0 silently, the result converts
    /// with `getInt()` (bools 0/1, pointer types 0), and callback errors
    /// pass through (`fPassErrors=true`, C4FindObject.cpp:947-956).
    fn value_for(
        &self,
        world: &impl WorldAccessor,
        object: &HostWorldObject,
    ) -> Result<i64, RuntimeError> {
        Ok(match self {
            SortCriterion::Distance { x, y } => {
                let position = object.position();
                let dx = i64::from(position.x - x);
                let dy = i64::from(position.y - y);
                dx * dx + dy * dy
            }
            SortCriterion::Random => RANDOM_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|context| i64::from(context.rng.borrow_mut().random(1 << 16)))
                    .unwrap_or(0)
            }),
            SortCriterion::Speed => {
                let velocity = object.velocity();
                let dx = i64::from(velocity.x);
                let dy = i64::from(velocity.y);
                dx * dx + dy * dy
            }
            SortCriterion::Mass => i64::from(
                world
                    .definition_metadata(object.definition_id())
                    .map(|metadata| metadata.mass)
                    .unwrap_or(0),
            ),
            SortCriterion::Value => i64::from(
                world
                    .definition_metadata(object.definition_id())
                    .map(|metadata| metadata.value)
                    .unwrap_or(0),
            ),
            SortCriterion::Func { name, pars } => {
                match call_world_object_function(object.id, name, pars) {
                    None => 0,
                    Some(result) => i64::from(result?.as_c4_int().unwrap_or(0)),
                }
            }
            SortCriterion::Reverse(_) | SortCriterion::Multiple(_) => 0,
        })
    }

    /// `C4SortObject::SortObjects` (C4FindObject.cpp:784-812): per-criterion
    /// value caches computed in collection order, then a stable sort with
    /// `Compare > 0` ⇒ ascending by value (smallest first).
    fn sort(&self, world: &impl WorldAccessor, ids: &mut [ObjectId]) -> Result<(), RuntimeError> {
        let keys = self.cache_keys(world, ids)?;
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by(|&a, &b| Self::compare_keys(&keys[a], &keys[b]));
        let sorted: Vec<ObjectId> = order.iter().map(|&index| ids[index]).collect();
        ids.copy_from_slice(&sorted);
        Ok(())
    }

    /// Per-object key vectors: flattened (criterion, direction) values so
    /// Reverse/Multiple compose like the C++ Compare chain.
    fn cache_keys(
        &self,
        world: &impl WorldAccessor,
        ids: &[ObjectId],
    ) -> Result<Vec<Vec<i64>>, RuntimeError> {
        let mut keys = vec![Vec::new(); ids.len()];
        self.fill_keys(world, ids, &mut keys, false)?;
        Ok(keys)
    }

    fn fill_keys(
        &self,
        world: &impl WorldAccessor,
        ids: &[ObjectId],
        keys: &mut [Vec<i64>],
        reverse: bool,
    ) -> Result<(), RuntimeError> {
        match self {
            SortCriterion::Reverse(child) => child.fill_keys(world, ids, keys, !reverse)?,
            SortCriterion::Multiple(children) => {
                for child in children {
                    child.fill_keys(world, ids, keys, reverse)?;
                }
            }
            _ => {
                let sign = if reverse { -1 } else { 1 };
                for (index, id) in ids.iter().enumerate() {
                    let value = match world.get_object(*id) {
                        Some(object) => self.value_for(world, &object)?,
                        None => 0,
                    };
                    keys[index].push(sign * value);
                }
            }
        }
        Ok(())
    }

    fn compare_keys(a: &[i64], b: &[i64]) -> std::cmp::Ordering {
        for (lhs, rhs) in a.iter().zip(b.iter()) {
            match lhs.cmp(rhs) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }

    /// The UNCACHED `Compare(obj1, obj2)` used by the single-result Find
    /// path (C4FindObject.cpp:834-842): `CompareGetValue` runs for obj1
    /// then obj2 in hardcoded order, returning `value2 - value1` (>0 ⇒
    /// obj1 sorts first). Reverse swaps the arguments
    /// (C4FindObject.cpp:856-859); Multiple returns the first nonzero
    /// child comparison (C4FindObject.cpp:885-895).
    fn compare_uncached(
        &self,
        world: &impl WorldAccessor,
        obj1: &HostWorldObject,
        obj2: &HostWorldObject,
    ) -> Result<i64, RuntimeError> {
        match self {
            SortCriterion::Reverse(child) => child.compare_uncached(world, obj2, obj1),
            SortCriterion::Multiple(children) => {
                for child in children {
                    let result = child.compare_uncached(world, obj1, obj2)?;
                    if result != 0 {
                        return Ok(result);
                    }
                }
                Ok(0)
            }
            _ => {
                let value1 = self.value_for(world, obj1)?;
                let value2 = self.value_for(world, obj2)?;
                Ok(value2 - value1)
            }
        }
    }
}

/// The single-result Find with a sort attached (C4FindObject.cpp:186-199):
/// a running best, replaced when the uncached `Compare(candidate, best)`
/// is positive. No PrepareCache — value functions (and `C4SO_Random`
/// draws) run per comparison.
fn find_first_with_sort(
    world: &impl WorldAccessor,
    condition: &FindCondition,
    sort: &SortCriterion,
) -> Result<Option<ObjectId>, RuntimeError> {
    if condition.is_impossible(world) {
        return Ok(None);
    }
    let mut best: Option<(ObjectId, HostWorldObject)> = None;
    for object_id in world.object_ids() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !object.status().is_active() {
            continue;
        }
        if !condition.check(world, &object)? {
            continue;
        }
        best = match best {
            None => Some((object_id, object)),
            Some((best_id, best_object)) => {
                if sort.compare_uncached(world, &object, &best_object)? > 0 {
                    Some((object_id, object))
                } else {
                    Some((best_id, best_object))
                }
            }
        };
    }
    Ok(best.map(|(id, _)| id))
}

/// `CreateCriterionsFromPars` (C4Script.cpp:1985-2034): each argument array
/// parses as a condition or sort; conditions AND together, sorts merge into
/// a Multiple; no conditions at all is a script error.
fn parse_criterions(args: &[Value]) -> Option<(FindCondition, Option<SortCriterion>)> {
    let mut conditions = Vec::new();
    let mut sorts = Vec::new();
    for arg in args {
        // The first nil parameter ends the criteria list
        // (`if (!Data) break;`, C4Script.cpp:1996).
        if matches!(arg, Value::Nil) {
            break;
        }
        match FindCondition::parse(arg) {
            ParsedCriterion::Condition(condition) => conditions.push(condition),
            ParsedCriterion::Sort(sort) => sorts.push(sort),
            ParsedCriterion::None => {}
        }
    }
    if conditions.is_empty() {
        return None;
    }
    let condition = if conditions.len() == 1 {
        conditions.into_iter().next().expect("one condition")
    } else {
        FindCondition::And(conditions)
    };
    let sort = match sorts.len() {
        0 => None,
        1 => sorts.into_iter().next(),
        _ => Some(SortCriterion::Multiple(sorts)),
    };
    Some((condition, sort))
}

/// Search over the main object list (C4FindObject::Find/FindMany,
/// C4FindObject.cpp:180-226). The sector-bounds traversal optimization (and
/// its sector-order result ordering) is still open — the main list is always
/// walked, which matches the C++ unbounded path.
fn find_condition_matches(
    world: &impl WorldAccessor,
    condition: &FindCondition,
) -> Result<Vec<ObjectId>, RuntimeError> {
    if condition.is_impossible(world) {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for object_id in world.object_ids() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !object.status().is_active() {
            continue;
        }
        if condition.check(world, &object)? {
            matches.push(object_id);
        }
    }
    Ok(matches)
}

/// Whether the criteria need the borrow-free Func evaluation path.
fn criterions_use_func(condition: &FindCondition, sort: Option<&SortCriterion>) -> bool {
    condition.uses_func() || sort.map(SortCriterion::uses_func).unwrap_or(false)
}

/// FnFindObject2 (C4Script.cpp:2052-2067).
fn find_object2(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some((condition, sort)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "FindObject: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, sort.as_ref()) {
        let Some(view) = snapshot_func_find_view() else {
            return Ok(Value::Nil);
        };
        if let Some(sort) = sort {
            return Ok(find_first_with_sort(&view, &condition, &sort)?
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        let mut matches = find_condition_matches(&view, &condition)?;
        retain_live_nested(&mut matches);
        return Ok(matches
            .first()
            .map(|id| object_reference_value(*id))
            .unwrap_or(Value::Nil));
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if let Some(sort) = sort {
            return Ok(find_first_with_sort(context, &condition, &sort)?
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        let matches = find_condition_matches(context, &condition)?;
        Ok(matches
            .first()
            .map(|id| object_reference_value(*id))
            .unwrap_or(Value::Nil))
    })
}

/// FnFindObjects array form (C4Script.cpp:2069-2084).
fn find_objects2(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some((condition, sort)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "FindObjects: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, sort.as_ref()) {
        let Some(view) = snapshot_func_find_view() else {
            return Ok(Value::Array(Vec::new()));
        };
        let mut matches = find_condition_matches(&view, &condition)?;
        // Pre-sort: erase objects deleted during Check
        // (C4FindObject.cpp:217-218).
        retain_live_nested(&mut matches);
        if let Some(sort) = sort {
            sort.sort(&view, &mut matches)?;
        }
        // Post-sort: objects deleted by sort callbacks keep their slot as
        // nil (CheckObjectStatusAfterSort, C4FindObject.cpp:223,372-375).
        return Ok(Value::Array(HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            matches
                .into_iter()
                .map(|id| {
                    if borrow
                        .as_ref()
                        .map(|context| context.nested_object_destroyed(id))
                        .unwrap_or(false)
                    {
                        Value::Nil
                    } else {
                        object_reference_value(id)
                    }
                })
                .collect()
        })));
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Array(Vec::new()));
        };
        let mut matches = find_condition_matches(context, &condition)?;
        if let Some(sort) = sort {
            sort.sort(context, &mut matches)?;
        }
        Ok(Value::Array(
            matches.into_iter().map(object_reference_value).collect(),
        ))
    })
}

/// FnObjectCount2 (C4Script.cpp:2036-2050).
fn object_count2(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some((condition, _)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "ObjectCount: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, None) {
        let Some(view) = snapshot_func_find_view() else {
            return Ok(Value::Int(0));
        };
        let matches = find_condition_matches(&view, &condition)?;
        return Ok(Value::Int(truncate_to_i32(matches.len() as u64)));
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(0));
        };
        if condition.is_ensured(context) {
            return Ok(Value::Int(truncate_to_i32(
                context.world_object_ids().len() as u64,
            )));
        }
        Ok(Value::Int(truncate_to_i32(
            find_condition_matches(context, &condition)?.len() as u64,
        )))
    })
}

fn find_object(args: &[Value]) -> Result<Value, RuntimeError> {
    find_object_cpp(args, "FindObject", None)
}

/// Shared FnFindObject search (C4Script.cpp:2113-2135) with an optional
/// owner filter injected by FindObjectOwner (C4Script.cpp:2137-2161).
fn find_object_cpp(
    args: &[Value],
    function: &str,
    owner_override: Option<i32>,
) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let mut params = FindObjectParams::parse_cpp_call(args, function, context.caller_scope())?;
        if let Some(owner) = owner_override {
            params.owner = owner;
        }
        let result = if params.is_closest_query() {
            find_object_closest(context, &params)
        } else {
            find_object_linear(context, &params)
        };
        Ok(match result {
            Some(id) => object_reference_value(id),
            None => Value::Nil,
        })
    })
}

fn find_object_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnFindObjectOwner (C4Script.cpp:2137-2161): FindObject with the
    // owner filter as the SECOND parameter; an owner that is neither a
    // valid player nor NO_OWNER returns nil before any search. The
    // remaining arguments shift by one; exclude/container are not
    // script-settable here (C++ passes caller-exclusion and null).
    if args.len() > 10 {
        return Err(RuntimeError::new(
            "FindObjectOwner: expected at most 10 arguments",
        ));
    }
    let owner = parse_optional_i32(args.get(1), "FindObjectOwner", "owner")?.unwrap_or(0);
    let owner_valid = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        borrow
            .as_ref()
            .map(|context| owner == OWNER_NONE || context.player_state(owner).is_some())
            .unwrap_or(false)
    });
    if !owner_valid {
        return Ok(Value::Nil);
    }
    let mut remapped: Vec<Value> = Vec::with_capacity(10);
    remapped.push(args.first().cloned().unwrap_or(Value::Nil)); // id
    for slot in 2..=5 {
        remapped.push(args.get(slot).cloned().unwrap_or(Value::Nil)); // x y wdt hgt
    }
    remapped.push(args.get(6).cloned().unwrap_or(Value::Nil)); // ocf
    remapped.push(args.get(7).cloned().unwrap_or(Value::Nil)); // action
    remapped.push(args.get(8).cloned().unwrap_or(Value::Nil)); // action target
    remapped.push(Value::Nil); // container (not script-settable here)
    remapped.push(args.get(9).cloned().unwrap_or(Value::Nil)); // find next
    find_object_cpp(&remapped, "FindObjectOwner", Some(owner))
}

fn find_object_linear(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let mut skip_until = params.find_next;
    for object_id in params.candidate_ids(world) {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if !params.matches_object(&object) {
            continue;
        }
        if params.matches_area(world, &object) {
            return Some(object_id);
        }
    }
    None
}

fn find_object_closest(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let reference = params.reference_distance(world);
    let mut best: Option<(ObjectId, i64)> = None;
    for object_id in params.candidate_ids(world) {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !params.matches_object(&object) {
            continue;
        }
        let distance = squared_distance(object.position(), params.x, params.y);
        if let Some(reference) = reference {
            if distance <= reference {
                continue;
            }
        }
        match best {
            None => best = Some((object_id, distance)),
            Some((_, best_distance)) if distance < best_distance => {
                best = Some((object_id, distance));
            }
            _ => {}
        }
    }
    best.map(|(id, _)| id)
}

/// C++ `FindObjects` is the array-criteria form (C4Script.cpp:7043); the
/// legacy fixed-parameter form predates it in this port and is kept for the
/// existing fixtures. Array first argument → C++ semantics.
fn find_objects_dispatch(args: &[Value]) -> Result<Value, RuntimeError> {
    if matches!(args.first(), Some(Value::Array(_))) {
        find_objects2(args)
    } else {
        find_objects(args)
    }
}

fn find_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    let params = FindObjectParams::parse(args)?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Array(Vec::new())),
        };
        let ids = if params.is_closest_query() {
            collect_closest_matches(context, &params)
        } else {
            collect_linear_matches(context, &params)
        };
        let values = ids
            .into_iter()
            .map(object_reference_value)
            .collect::<Vec<_>>();
        Ok(Value::Array(values))
    })
}

fn object_count(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnObjectCount (C4Script.cpp:2085-2111): the FindObject layout with
    // iOwner instead of pFindNext as the 10th parameter; an owner of 0
    // becomes ANY_OWNER ("incomplete useless implementation").
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(0)),
        };
        let mut params = FindObjectParams::parse_cpp_call(
            &args[..args.len().min(9)],
            "ObjectCount",
            context.caller_scope(),
        )?;
        let owner = parse_optional_i32(args.get(9), "ObjectCount", "owner")?.unwrap_or(0);
        params.owner = if owner == 0 { OWNER_ANY } else { owner };
        let matches_len = if params.is_closest_query() {
            collect_closest_matches(context, &params).len()
        } else {
            collect_linear_matches(context, &params).len()
        };
        Ok(Value::Int(truncate_to_i32(matches_len as u64)))
    })
}

fn collect_linear_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let mut matches = Vec::new();
    let mut skip_until = params.find_next;
    for object_id in params.candidate_ids(world) {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if params.matches_object(&object) && params.matches_area(world, &object) {
            matches.push(object_id);
        }
    }
    matches
}

fn collect_closest_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let reference = params.reference_distance(world);
    let mut matches = Vec::new();
    for (order_index, object_id) in params.candidate_ids(world).into_iter().enumerate() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !params.matches_object(&object) {
            continue;
        }
        let distance = squared_distance(object.position(), params.x, params.y);
        if let Some(reference) = reference {
            if distance <= reference {
                continue;
            }
        }
        matches.push((distance, order_index, object_id));
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    matches.into_iter().map(|(_, _, id)| id).collect()
}

fn set_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetDir expects at least 1 argument: direction",
        ));
    }

    let raw_direction = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetDir: expected int or nil for direction, got {}",
                other.type_name()
            )))
        }
    };

    let direction = match Direction::from_script_value(raw_direction) {
        Some(direction) => direction,
        None => return Ok(Value::Bool(false)),
    };

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetDir requires an active engine context"))?;
        // FnSetDir works on ANY object (`if (!pObj) pObj = cthr->Obj`) —
        // GoldRush's init calls SetDir(1, pHorse) from the scenario
        // scope. Foreign writes stage into the target's nested scope.
        let active = context.object_context().map(|object| object.id());
        if let Some(target) = target_id {
            if Some(target) != active {
                if !context.nested_objects.contains_key(&target) {
                    let Some(world_object) = context.get_world_object(target) else {
                        return Ok(Value::Bool(false));
                    };
                    let Some((scope, local_vars)) = context.nested_scope_for(&world_object)
                    else {
                        return Ok(Value::Bool(false));
                    };
                    context
                        .nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
                if !context.nested_order.contains(&target) {
                    context.nested_order.push(target);
                }
                let state = context
                    .nested_objects
                    .get_mut(&target)
                    .expect("scope just ensured");
                let scope = &mut state.scope;
                // The same C4Object::SetDir gates as the self path; the
                // TurnAction fires through the staged action update.
                let action_name = scope.effective_action_name().to_string();
                if scope.action_library.is_idle_action(&action_name) {
                    return Ok(Value::Bool(false));
                }
                let directions = scope.action_library.directions_for(&action_name) as i32;
                if direction.to_script_value() < 0
                    || direction.to_script_value() >= directions
                {
                    return Ok(Value::Bool(false));
                }
                if scope.direction() != direction {
                    if let Some(turn_action) = scope
                        .action_library
                        .turn_action_for(&action_name)
                        .map(str::to_string)
                    {
                        if scope.action_library.contains(&turn_action) {
                            let update = scope
                                .pending_update
                                .action
                                .get_or_insert_with(ActionUpdate::default);
                            update.set_name(turn_action.clone());
                            update.set_force(false);
                            scope.update_effective_action(&turn_action);
                        }
                    }
                }
                scope.set_direction(direction);
                return Ok(Value::Bool(true));
            }
        }
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        // C4Object::SetDir (C4Object.cpp:4225-4245): a no-op on idle
        // objects and out-of-range directions (Directions defaults 1 —
        // single-direction actions reject SetDir(1)); a CHANGE fires the
        // action's TurnAction through SetActionByName first.
        let action_name = object.effective_action_name().to_string();
        if object.action_library.is_idle_action(&action_name) {
            return Ok(Value::Bool(false));
        }
        let directions = object.action_library.directions_for(&action_name) as i32;
        if direction.to_script_value() < 0 || direction.to_script_value() >= directions {
            return Ok(Value::Bool(false));
        }
        let turn_action = (object.direction() != direction)
            .then(|| {
                object
                    .action_library
                    .turn_action_for(&action_name)
                    .map(str::to_string)
            })
            .flatten();
        drop(borrow);
        if let Some(turn_action) = turn_action {
            let _ = set_action(&[Value::String(turn_action)])?;
        }
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let context = borrow
                .as_mut()
                .ok_or_else(|| RuntimeError::new("SetDir requires an active engine context"))?;
            if let Some(object) = context.object_context_mut() {
                object.set_direction(direction);
            }
            Ok(Value::Bool(true))
        })
    })
}

fn get_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.direction().to_script_value()))
    })
}

fn set_r(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetR expects at least 1 argument: rotation",
        ));
    }

    let rotation = value_to_i32(&args[0], "SetR", "rotation")?;

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetR", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetR: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetR requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_rotation(rotation);
        Ok(Value::Bool(true))
    })
}

fn get_r(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new("GetR expects at most 1 argument: target"));
    }

    let target_id =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetR", "target")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(object.rotation()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.rotation.rem_euclid(360)));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(object.rotation()))
    })
}

fn set_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetComDir expects at least 1 argument: command direction",
        ));
    }

    let raw_direction = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetComDir: expected int or nil for command direction, got {}",
                other.type_name()
            )))
        }
    };

    let command_direction = match CommandDirection::from_script_value(raw_direction) {
        Some(direction) => direction,
        None => return Ok(Value::Bool(false)),
    };

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetComDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetComDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetComDir requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_command_direction(command_direction);
        Ok(Value::Bool(true))
    })
}

fn get_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetComDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetComDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.command_direction().to_script_value()))
    })
}

fn set_command(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnSetCommand leads with the object slot (pObj, szCommand, ...;
    // C4Script.cpp:840-844); 0/nil means the calling object. The
    // name-first form stays for the command-DSL fixtures. A FOREIGN
    // target re-dispatches through the reentrancy seam so the command
    // stack write folds with the target's own nested outcome (GoldRush's
    // StopClonk drives other clonks, Helpers.c:94).
    let mut args = args;
    let mut leading_target: Option<ObjectId> = None;
    let leads_with_object_slot = matches!(
        (args.first(), args.get(1)),
        (Some(Value::Object(_) | Value::Proplist(_)), _)
            | (Some(Value::Nil | Value::Int(0)), Some(Value::String(_)))
    );
    if leads_with_object_slot {
        leading_target = parse_object_reference_argument(&args[0], "SetCommand", "target")?;
        args = &args[1..];
    }
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetCommand expects at least 1 argument: command name",
        ));
    }
    if let Some(target) = leading_target {
        if active_object_id() != Some(target) {
            return match call_world_object_function(target, "SetCommand", args) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetCommand requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        let command_name = match &args[0] {
            Value::String(name) if !name.is_empty() => name.clone(),
            Value::String(_) | Value::Nil => {
                object.clear_command_stack();
                return Ok(Value::Bool(false));
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "SetCommand: expected string for command name, got {}",
                    other.type_name()
                )))
            }
        };

        let command_id = match CommandId::from_name(&command_name) {
            Some(id) => id,
            None => {
                object.clear_command_stack();
                return Ok(Value::Bool(false));
            }
        };

        let request = parse_command_request(command_id, args, "SetCommand")?;
        object.clear_command_stack();
        let success = object.push_command_front(request);
        Ok(Value::Bool(success))
    })
}

fn add_command(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnAddCommand leads with the object slot (pObj, szCommand, ...;
    // C4Script.cpp:870-874); 0/nil means the calling object. The
    // name-first form stays for the command-DSL fixtures. Like
    // SetCommand, only the SELF form is dispatchable (documented gap).
    let mut args = args;
    let mut leading_target: Option<ObjectId> = None;
    let leads_with_object_slot = matches!(
        (args.first(), args.get(1)),
        (Some(Value::Object(_) | Value::Proplist(_)), _)
            | (Some(Value::Nil | Value::Int(0)), Some(Value::String(_)))
    );
    if leads_with_object_slot {
        leading_target = parse_object_reference_argument(&args[0], "AddCommand", "target")?;
        args = &args[1..];
    }
    if args.is_empty() {
        return Err(RuntimeError::new(
            "AddCommand expects at least 1 argument: command name",
        ));
    }

    let command_name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddCommand: expected string for command name, got {}",
                other.type_name()
            )))
        }
    };

    let command_id = match CommandId::from_name(&command_name) {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };

    let request = parse_command_request(command_id, args, "AddCommand")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("AddCommand requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        if let Some(target) = leading_target {
            if target != object.id() {
                tracing::warn!(
                    ?target,
                    "AddCommand on a FOREIGN object is not dispatchable yet; ignoring"
                );
                return Ok(Value::Bool(false));
            }
        }

        let success = object.push_command_front(request);
        Ok(Value::Bool(success))
    })
}

fn append_command(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "AppendCommand expects at least 1 argument: command name",
        ));
    }

    let command_name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AppendCommand: expected string for command name, got {}",
                other.type_name()
            )))
        }
    };

    let command_id = match CommandId::from_name(&command_name) {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };

    let request = parse_command_request(command_id, args, "AppendCommand")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("AppendCommand requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        let success = object.push_command_back(request);
        Ok(Value::Bool(success))
    })
}

enum PositionComponent {
    X,
    Y,
}

impl PositionComponent {
    fn function_name(&self) -> &'static str {
        match self {
            PositionComponent::X => "GetX",
            PositionComponent::Y => "GetY",
        }
    }

    fn extract(&self, position: Vector2) -> i32 {
        match self {
            PositionComponent::X => position.x,
            PositionComponent::Y => position.y,
        }
    }
}

fn get_position_component(
    args: &[Value],
    component: PositionComponent,
) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(format!(
            "{} expects at most 1 argument: target",
            component.function_name()
        )));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, component.function_name(), "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let position = object.effective_position();
                    return Ok(Value::Int(component.extract(position)));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let position = other.position();
                return Ok(Value::Int(component.extract(position)));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let position = object.effective_position();
        Ok(Value::Int(component.extract(position)))
    })
}

fn object_distance(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "ObjectDistance expects 1 or 2 arguments: other, object",
        ));
    }

    let other_id = match parse_object_reference_argument(&args[0], "ObjectDistance", "other")? {
        Some(id) => id,
        None => return Ok(Value::Nil),
    };

    let mut reference_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(1) {
        reference_id = parse_object_reference_argument(arg, "ObjectDistance", "object")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let locate_position = |id: ObjectId| -> Option<Vector2> {
            if let Some(object) = context.object_context() {
                if object.id() == id {
                    return Some(object.effective_position());
                }
            }
            context.get_world_object(id).map(|object| object.position())
        };

        let anchor_position = if let Some(id) = reference_id {
            locate_position(id)
        } else {
            context
                .object_context()
                .map(|object| object.effective_position())
        };

        let anchor_position = match anchor_position {
            Some(position) => position,
            None => return Ok(Value::Nil),
        };

        let other_position = match locate_position(other_id) {
            Some(position) => position,
            None => return Ok(Value::Nil),
        };

        let distance = integer_distance(
            anchor_position.x,
            anchor_position.y,
            other_position.x,
            other_position.y,
        );
        Ok(Value::Int(distance))
    })
}

enum VelocityComponent {
    X,
    Y,
}

impl VelocityComponent {
    fn get_function_name(&self) -> &'static str {
        match self {
            VelocityComponent::X => "GetXDir",
            VelocityComponent::Y => "GetYDir",
        }
    }

    fn set_function_name(&self) -> &'static str {
        match self {
            VelocityComponent::X => "SetXDir",
            VelocityComponent::Y => "SetYDir",
        }
    }

    fn extract_fixed(&self, velocity: FixedVec2) -> C4Fixed {
        match self {
            VelocityComponent::X => velocity.x,
            VelocityComponent::Y => velocity.y,
        }
    }

    fn assign_fixed(&self, velocity: &mut FixedVec2, value: C4Fixed) {
        match self {
            VelocityComponent::X => velocity.x = value,
            VelocityComponent::Y => velocity.y = value,
        }
    }
}

fn get_velocity_component(
    args: &[Value],
    component: VelocityComponent,
) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(format!(
            "{} expects at most 2 arguments: target, precision",
            component.get_function_name()
        )));
    }

    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id =
                parse_object_reference_argument(arg, component.get_function_name(), "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, component.get_function_name(), "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.get_function_name()
        )));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let effective_precision = normalise_precision(precision);
        let fetch_velocity = |fixed_velocity: FixedVec2| {
            // C++ GetXDir/GetYDir return fixtoi(xdir/ydir, prec). `C4Script.cpp:1167`.
            let component_value = component.extract_fixed(fixed_velocity);
            Value::Int(fixtoi_prec(component_value, effective_precision))
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(fetch_velocity(object.fixed_velocity()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                // World objects only carry whole-pixel velocity from their
                // snapshot; reconstruct fixed via itofix (sub-pixel fidelity for
                // foreign objects awaits the snapshot work, task B).
                let velocity = other.velocity();
                return Ok(fetch_velocity(FixedVec2::from_ints(velocity.x, velocity.y)));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(fetch_velocity(object.fixed_velocity()))
    })
}

fn set_velocity_component(
    args: &[Value],
    component: VelocityComponent,
) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(format!(
            "{} expects at least 1 argument: value",
            component.set_function_name()
        )));
    }

    let value = value_to_i32(&args[0], component.set_function_name(), "value")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id =
                parse_object_reference_argument(arg, component.set_function_name(), "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, component.set_function_name(), "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.set_function_name()
        )));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new(format!(
                "{} requires an active engine context",
                component.set_function_name()
            ))
        })?;

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        // C++ SetXDir/SetYDir set ONLY xdir/ydir = itofix(value, prec)
        // (default precision 10, C4Script.cpp:697-732) — the other
        // component keeps its full sub-pixel value. A whole-vector write
        // here would quantize it through the scope's int-seeded mirror
        // (the GoldRush snake's SetXDir(0) turned ydir 2.6 into 3.0).
        object.set_fixed_velocity_component(
            component,
            itofix_prec(value, normalise_precision(precision)),
        );
        Ok(Value::Bool(true))
    })
}

fn get_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::X)
}

fn get_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::Y)
}

fn set_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::X)
}

fn set_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::Y)
}

fn set_r_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnSetRDir(value, [target], [precision = 10]) sets rdir = itofix(value,
    // precision), a fractional `C4Fixed` angular velocity. `C4Script.cpp:710`.
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetRDir expects at least 1 argument: value",
        ));
    }

    let value = value_to_i32(&args[0], "SetRDir", "value")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "SetRDir", "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, "SetRDir", "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetRDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetRDir requires an active engine context"))?;

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_rotation_velocity(itofix_prec(value, normalise_precision(precision)));
        Ok(Value::Bool(true))
    })
}

fn get_r_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnGetRDir([target], [precision = 10]) returns fixtoi(rdir, precision).
    // `C4Script.cpp` GetRDir.
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetRDir expects at most 2 arguments: target, precision",
        ));
    }

    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "GetRDir", "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, "GetRDir", "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetRDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let effective_precision = normalise_precision(precision);
        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(fixtoi_prec(
                        object.rotation_velocity(),
                        effective_precision,
                    )));
                }
            }
            // Foreign objects do not expose `rdir` to scripts yet (no snapshot
            // field); report nil rather than a fabricated value.
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(Value::Int(fixtoi_prec(
            object.rotation_velocity(),
            effective_precision,
        )))
    })
}

fn get_x(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::X)
}

fn get_y(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::Y)
}

fn get_id(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetID expects at most 1 argument: target object",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetID", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            // Lookup object by ID and return its definition_id
            if let Some(world_object) = context.get_world_object(target) {
                return Ok(Value::C4Id(world_object.definition_id().to_string()));
            }
            // If target object not found, return nil
            return Ok(Value::Nil);
        }

        // No argument provided - return current object's definition_id
        if let Some(object) = context.object_context() {
            let object_id = object.id();
            if let Some(world_object) = context.get_world_object(object_id) {
                return Ok(Value::C4Id(world_object.definition_id().to_string()));
            }
            // The world snapshot predates the object during its own
            // materialize-time Initialize — the scope knows its def.
            if let Some(definition_id) = object.definition_id.clone() {
                return Ok(Value::C4Id(definition_id));
            }
        }

        Ok(Value::Nil)
    })
}

fn apply_position_bounds(
    desired: Vector2,
    vertices: &[ObjectVertex],
    landscape: Option<&Landscape>,
) -> Vector2 {
    let mut bounded = desired;
    let Some(landscape) = landscape else {
        return bounded;
    };

    let width = landscape.width() as i32;
    if width > 0 {
        let (mut min_allowed, mut max_allowed) = if vertices.is_empty() {
            (0, width.saturating_sub(1))
        } else {
            vertices
                .iter()
                .fold((i32::MIN, i32::MAX), |(min_acc, max_acc), vertex| {
                    (
                        min_acc.max(-vertex.x),
                        max_acc.min(width.saturating_sub(1).saturating_sub(vertex.x)),
                    )
                })
        };

        if min_allowed == i32::MIN {
            min_allowed = 0;
        }
        if max_allowed == i32::MAX {
            max_allowed = width.saturating_sub(1);
        }

        if min_allowed <= max_allowed {
            bounded.x = bounded.x.clamp(min_allowed, max_allowed);
        } else {
            bounded.x = bounded.x.clamp(0, width.saturating_sub(1));
        }
    }

    let min_y_allowed = if vertices.is_empty() {
        0
    } else {
        vertices.iter().map(|vertex| -vertex.y).max().unwrap_or(0)
    };

    let mut max_y_allowed = i32::MAX;
    if vertices.is_empty() {
        if let Some(surface_y) = landscape.surface_height(bounded.x) {
            max_y_allowed = surface_y;
        }
    } else {
        for vertex in vertices {
            let world_x = bounded.x.saturating_add(vertex.x);
            if let Some(surface_y) = landscape.surface_height(world_x) {
                max_y_allowed = max_y_allowed.min(surface_y - vertex.y);
            }
        }
    }

    if max_y_allowed < min_y_allowed {
        max_y_allowed = min_y_allowed;
    }

    bounded.y = bounded.y.clamp(min_y_allowed, max_y_allowed);
    bounded
}

fn set_position(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "SetPosition expects at least 2 arguments: x, y",
        ));
    }

    let x = value_to_i32(&args[0], "SetPosition", "x")?;
    let y = value_to_i32(&args[1], "SetPosition", "y")?;

    let mut index = 2;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetPosition", "target")?;
        index += 1;
    }

    let mut check_bounds = false;
    if let Some(arg) = args.get(index) {
        check_bounds = match arg {
            Value::Bool(value) => *value,
            Value::Int(value) => *value != 0,
            Value::Nil => false,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetPosition: expected bool for check_bounds, got {}",
                    other.type_name()
                )))
            }
        };
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetPosition: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetPosition requires an active engine context"))?;

        let landscape_snapshot = if check_bounds {
            context.landscape_ref().cloned()
        } else {
            None
        };

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        let mut position = Vector2::new(x, y);
        if check_bounds {
            let vertices: Vec<ObjectVertex> = object.vertices().to_vec();
            position = apply_position_bounds(position, &vertices, landscape_snapshot.as_ref());
        }

        object.set_position(position);
        Ok(Value::Bool(true))
    })
}

fn create_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateObject expects at least 1 argument: definition",
        ));
    }

    // FnCreateObject takes a C4ID (C4Script.cpp:1892); our resources address
    // definitions by their id string, so id values and strings coincide.
    let definition = match &args[0] {
        Value::String(name) | Value::C4Id(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::C4Id(_) | Value::Nil | Value::Int(0) => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateObject: expected id for definition, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let x_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateObject", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateObject", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let mut owner_override: Option<i32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                owner_override = Some(*value);
                index += 1;
            }
            Value::Nil => {
                owner_override = Some(OWNER_NONE);
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "CreateObject: expected int or nil for owner, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateObject: additional arguments are not supported",
        ));
    }

    let created = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateObject requires an active engine context"))?;

        // C4Id2Def failure: no object, silent nullptr (C4Game.cpp:1146).
        if context.world.definition_known(&definition) == Some(false) {
            return Ok(Value::Nil);
        }

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                name: String::new(),
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            });
        let definition_category = metadata.category;

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);
        // An absent owner argument is the DEFAULTED C4ValueInt 0, not
        // NO_OWNER — FnCreateObject only substitutes the caller's owner
        // for NONSTRICT scripts (C4Script.cpp FnCreateObject: `if
        // (!cthr->Caller || ...Strict == NONSTRICT) iOwner =
        // cthr->Obj->Owner`); GoldRush's global `CreateObject(NOPC)`
        // lands on owner 0 in C++.
        let owner = owner_override.unwrap_or(0);
        // C4Game::NewObject's DoCon(fInitial) bottom-growth adjust runs
        // BEFORE the next script line executes (C4Object.cpp:1401-1470):
        // the created object's center is y - (Shape.Hgt + Shape.y), and
        // sibling calls in the same script (ConnectWagon's beam offsets)
        // read the FINAL position.
        let raw_position = Vector2::new(
            base_position.x.saturating_add(x_offset),
            base_position.y.saturating_add(y_offset),
        );
        let position = Vector2::new(
            raw_position.x,
            crate::docon_initial_center_y(
                metadata.shape,
                metadata.stretch_growth,
                metadata.line,
                crate::FULL_CON,
                raw_position.y,
            ),
        );

        let id = context.allocate_object_id();

        let mut spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
            .with_id(id);
        // "Set initial controller to creating controller, so more
        // complicated cause-effect-chains can be traced back to the
        // causing player" (FnCreateObject, C4Script.cpp:1905-1906).
        let creator_controller = context
            .object_context()
            .map(ObjectScopeContext::controller)
            .filter(|value| *value > OWNER_NONE);
        if let Some(controller) = creator_controller {
            spawn = spawn.with_controller(controller);
        }
        // Creation callbacks run synchronously below - materialization
        // must not repeat Initialize (C4Game.cpp:1117-1127).
        spawn.initialized = true;
        spawn.position_adjusted = true;
        // The DoCon initial adjust moves the INT y only (C++ leaves fix_y
        // at the given center) — carry the raw fixed alongside.
        if position.y != raw_position.y {
            spawn.fixed_position = Some(FixedVec2::from_ints(raw_position.x, raw_position.y));
        }

        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            true,
            ObjectStatus::Normal,
            false,
            FULL_CON,
        );
        let preview = HostWorldObject::with_category(
            id,
            definition,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            owner,
            definition_category,
            0,
            FULL_CON,
            0,
            position,
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
            None,
        )
        .with_ocf(preview_ocf)
        // A callable scope for nested calls on the fresh object — C++
        // creates objects live mid-call (Game.CreateObject), so scripts
        // arrow-call them immediately (GoldRush: pObj->SetAI right after
        // CreateObject). The spawn stays authoritative; nested outcomes
        // fold only touched fields.
        .with_full_state(Rc::new(crate::preview_spawn_state(
            position,
            owner,
            creator_controller.unwrap_or(owner),
            definition_category,
            FULL_CON,
            metadata.vertices.clone(),
        )));

        context.register_spawn(spawn, preview);
        Ok(object_reference_value(id))
    });

    // C4Game::NewObject runs PSF_Construction and the initial DoCon's
    // PSF_Initialize INSIDE FnCreateObject (C4Game.cpp:1117-1127): the
    // new object's script side effects exist the moment CreateObject
    // returns (GoldRush bandits' appended Life effect precedes SetAI's
    // RemoveEffect). Both are fail-silent when the def lacks them; errors
    // log and continue like C4Object::Call(fPassError=false).
    if let Ok(value @ Value::Object(_)) = &created {
        if let Some(target) = object_id_from_value(value) {
            for callback in ["Construction", "Initialize"] {
                if let Some(Err(error)) = call_world_object_own_function(target, callback, &[]) {
                    tracing::warn!(
                        id = target.as_u64(),
                        callback,
                        %error,
                        "creation callback failed; continuing like C++ fail-safe Call"
                    );
                }
            }
        }
    }
    created
}

fn create_construction(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateConstruction expects at least 1 argument: definition",
        ));
    }

    // FnCreateConstruction takes a C4ID (C4Script.cpp:1911-1912); our
    // resources address definitions by their id string, so id values and
    // strings coincide.
    let definition = match &args[0] {
        Value::String(name) | Value::C4Id(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::C4Id(_) | Value::Nil | Value::Int(0) => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateConstruction: expected id for definition, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let x_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let mut owner_override: Option<i32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                owner_override = Some(*value);
                index += 1;
            }
            Value::Nil => {
                owner_override = Some(OWNER_NONE);
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "CreateConstruction: expected int or nil for owner, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let completion_percent = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "completion")?;
        index += 1;
        value
    } else {
        0
    };

    let _terrain_flag = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "CreateConstruction", "terrain")?;
        index += 1;
        flag
    } else {
        false
    };

    let check_site = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "CreateConstruction", "check_site")?;
        index += 1;
        flag
    } else {
        false
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateConstruction: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("CreateConstruction requires an active engine context")
        })?;

        // C4Id2Def failure: no site, silent nullptr (C4Game.cpp:1183).
        if context.world.definition_known(&definition) == Some(false) {
            return Ok(Value::Nil);
        }

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                name: String::new(),
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: true,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            });
        let definition_category = metadata.category;

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);
        let base_owner = context
            .object_context()
            .map(|object| object.owner())
            .unwrap_or(OWNER_NONE);

        let owner = owner_override.unwrap_or(base_owner);
        let position = Vector2::new(
            base_position.x.saturating_add(x_offset),
            base_position.y.saturating_add(y_offset),
        );

        let completion = completion_percent.clamp(0, 100);
        let construction_value = ((i64::from(completion) * i64::from(FULL_CON)) / 100)
            .clamp(0, i64::from(FULL_CON)) as i32;

        if check_site && !construction_check(context, &definition, &metadata, position)? {
            return Ok(Value::Nil);
        }

        let id = context.allocate_object_id();

        let mut spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
            .with_construction(construction_value)
            .with_id(id);
        // The creating controller rides onto the site (FnCreateConstruction,
        // C4Script.cpp:1932-1933).
        let creator_controller = context
            .object_context()
            .map(ObjectScopeContext::controller)
            .filter(|value| *value > OWNER_NONE);
        if let Some(controller) = creator_controller {
            spawn = spawn.with_controller(controller);
        }

        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            true,
            ObjectStatus::Normal,
            false,
            construction_value,
        );
        let preview = HostWorldObject::with_category(
            id,
            definition,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            owner,
            definition_category,
            0,
            construction_value,
            0,
            position,
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
            None,
        )
        .with_ocf(preview_ocf)
        .with_full_state(Rc::new(crate::preview_spawn_state(
            position,
            owner,
            creator_controller.unwrap_or(owner),
            definition_category,
            construction_value,
            metadata.vertices.clone(),
        )));

        context.register_spawn(spawn, preview);
        Ok(object_reference_value(id))
    })
}

fn construction_check(
    context: &EffectHostContext,
    definition_id: &str,
    metadata: &DefinitionMetadata,
    position: Vector2,
) -> Result<bool, RuntimeError> {
    if !metadata.constructable {
        return Ok(false);
    }

    let (raw_width, raw_height) = metadata
        .shape
        .map(|rect| (rect.width, rect.height))
        .unwrap_or((20, 40));
    let width = raw_width.max(1);
    let height = raw_height.max(1);
    let effective_height = height.saturating_sub(metadata.construction_offset).max(1);

    let rect_left = position.x - width / 2;
    let rect_right = rect_left + width;
    let rect_top = position.y - effective_height;
    let rect_bottom = position.y;

    let Some(landscape) = context.landscape_ref() else {
        return Ok(true);
    };

    let landscape_width = landscape.width() as i32;
    if rect_left < 0 || rect_right > landscape_width {
        return Ok(false);
    }

    let mut solid_count: i32 = 0;
    let mut support_count: i32 = 0;
    for column in rect_left..rect_right {
        let surface = match landscape.surface_height(column) {
            Some(height) => height,
            None => return Ok(false),
        };
        let overlap_start = rect_top.max(surface);
        let overlap_height = (rect_bottom - overlap_start).max(0);
        solid_count = solid_count.saturating_add(overlap_height);

        let support_start = rect_bottom.max(surface);
        let support_height = (rect_bottom + 5 - support_start).max(0);
        support_count = support_count.saturating_add(support_height);
    }

    let area_threshold = ((i64::from(width) * i64::from(effective_height)) / 20)
        .clamp(0, i64::from(i32::MAX)) as i32;
    if solid_count > area_threshold {
        return Ok(false);
    }

    if support_count < width.saturating_mul(2) {
        return Ok(false);
    }

    let overlap_mask = metadata.category & CATEGORY_SORT_LIMIT;
    if overlap_mask == 0 {
        return Ok(true);
    }

    let current_object_id = context.object_context().map(|object| object.id());
    for object_id in context.world_object_ids() {
        let Some(other) = context.get_world_object(object_id) else {
            continue;
        };
        if Some(other.id) == current_object_id {
            continue;
        }
        if !other.is_present() || !other.status().is_active() {
            continue;
        }
        if other.container().is_some() {
            continue;
        }
        if other.category() & overlap_mask & CATEGORY_SORT_LIMIT == 0 {
            continue;
        }
        let other_metadata = if other.definition_id() == definition_id {
            Some(metadata)
        } else {
            context.definition_metadata(other.definition_id())
        };
        if let Some(bounds) = compute_object_bounds(&other, other_metadata) {
            if rectangles_overlap((rect_left, rect_top, rect_right, rect_bottom), bounds) {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

fn compute_object_bounds(
    object: &HostWorldObject,
    metadata: Option<&DefinitionMetadata>,
) -> Option<(i32, i32, i32, i32)> {
    if let Some(meta) = metadata {
        if let Some(shape) = meta.shape {
            let position = object.position();
            let left = position.x + shape.x;
            let top = position.y + shape.y;
            let right = left + shape.width;
            let bottom = top + shape.height;
            return Some((left, top, right, bottom));
        }
    }

    let vertices = object.vertices();
    if vertices.is_empty() {
        return None;
    }

    let mut min_x = vertices[0].x;
    let mut max_x = min_x;
    let mut min_y = vertices[0].y;
    let mut max_y = min_y;
    for vertex in vertices.iter().skip(1) {
        if vertex.x < min_x {
            min_x = vertex.x;
        }
        if vertex.x > max_x {
            max_x = vertex.x;
        }
        if vertex.y < min_y {
            min_y = vertex.y;
        }
        if vertex.y > max_y {
            max_y = vertex.y;
        }
    }

    let position = object.position();
    Some((
        position.x + min_x,
        position.y + min_y,
        position.x + max_x,
        position.y + max_y,
    ))
}

fn rectangles_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let (a_left, a_top, a_right, a_bottom) = a;
    let (b_left, b_top, b_right, b_bottom) = b;
    a_left < b_right && a_right > b_left && a_top < b_bottom && a_bottom > b_top
}

fn create_particle(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateParticle expects at least 1 argument: name",
        ));
    }

    let definition = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateParticle: expected string for name, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let x = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let x_dir = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "xdir")?;
        index += 1;
        value
    } else {
        0
    };

    let y_dir = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "ydir")?;
        index += 1;
        value
    } else {
        0
    };

    let parameter_a = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "a")?;
        index += 1;
        value
    } else {
        0
    };

    let life_raw = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "b")?;
        index += 1;
        value
    } else {
        0
    };

    let mut target_object: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_object = parse_object_reference_argument(arg, "CreateParticle", "object")?;
        index += 1;
    }

    let mut back = false;
    if let Some(arg) = args.get(index) {
        back = value_to_bool(arg, "CreateParticle", "back")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateParticle: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateParticle requires an active engine context"))?;

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);

        let world_x = base_position.x.saturating_add(x);
        let world_y = base_position.y.saturating_add(y);

        let layer = if let Some(target) = target_object {
            let world_object = match context.get_world_object(target) {
                Some(object) => object,
                None => return Ok(Value::Bool(false)),
            };
            if !world_object.status().is_active() {
                return Ok(Value::Bool(false));
            }
            if back {
                ParticleLayer::ObjectBack(target)
            } else {
                ParticleLayer::ObjectFront(target)
            }
        } else {
            ParticleLayer::Global
        };

        // GetDef failure → false (C4Script.cpp:4874)
        if context.particle_def_known(&definition) == Some(false) {
            return Ok(Value::Bool(false));
        }

        let config = ParticleConfig {
            definition_id: definition,
            position: FloatVector2::new(world_x as f32, world_y as f32),
            velocity: FloatVector2::new(x_dir as f32 / 10.0, y_dir as f32 / 10.0),
            life: life_raw.max(0),
            parameter_a: parameter_a as f32 / 10.0,
            parameter_b: life_raw,
            layer,
        };

        context.register_particle(ParticleCommand::Create(config));
        Ok(Value::Bool(true))
    })
}

/// FnCastAParticles (C4Script.cpp:4881-4898), shared by CastParticles and
/// CastBackParticles. Args: name, amount, level, x, y, a0, a1, b0, b1, obj.
fn cast_a_particles(args: &[Value], back: bool, fn_name: &str) -> Result<Value, RuntimeError> {
    let definition = match args.first() {
        Some(Value::String(name)) if !name.is_empty() => name.clone(),
        Some(Value::String(_)) | Some(Value::Nil) | None => return Ok(Value::Bool(false)),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "{fn_name}: expected string for name, got {}",
                other.type_name()
            )))
        }
    };

    let int_arg = |index: usize, label: &str| -> Result<i32, RuntimeError> {
        args.get(index)
            .map(|arg| value_to_i32(arg, fn_name, label))
            .transpose()
            .map(|value| value.unwrap_or(0))
    };
    let amount = int_arg(1, "amount")?;
    let level = int_arg(2, "level")?;
    let x = int_arg(3, "x")?;
    let y = int_arg(4, "y")?;
    let a0 = int_arg(5, "a0")?;
    let a1 = int_arg(6, "a1")?;
    let b0 = int_arg(7, "b0")? as u32;
    let b1 = int_arg(8, "b1")? as u32;

    let target_object = args
        .get(9)
        .map(|arg| parse_object_reference_argument(arg, fn_name, "object"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new(format!("{fn_name} requires an active engine context"))
        })?;

        // safety: pObj && !pObj->Status → false (C4Script.cpp:4884)
        let layer = if let Some(target) = target_object {
            let world_object = match context.get_world_object(target) {
                Some(object) => object,
                None => return Ok(Value::Bool(false)),
            };
            if !world_object.status().is_active() {
                return Ok(Value::Bool(false));
            }
            if back {
                ParticleLayer::ObjectBack(target)
            } else {
                ParticleLayer::ObjectFront(target)
            }
        } else {
            ParticleLayer::Global
        };

        // GetDef failure → false (C4Script.cpp:4893)
        if context.particle_def_known(&definition) == Some(false) {
            return Ok(Value::Bool(false));
        }

        // local offset (C4Script.cpp:4886-4890)
        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);

        context.register_particle(ParticleCommand::Cast {
            definition_id: definition,
            amount,
            x: base_position.x.saturating_add(x) as f32,
            y: base_position.y.saturating_add(y) as f32,
            level,
            a0: a0 as f32 / 10.0,
            b0,
            a1: a1 as f32 / 10.0,
            b1,
            layer,
        });
        Ok(Value::Bool(true))
    })
}

fn cast_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    cast_a_particles(args, false, "CastParticles")
}

fn cast_back_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    cast_a_particles(args, true, "CastBackParticles")
}

/// FnPushParticles (C4Script.cpp:4910-4923): name nil → push all particles;
/// a named def that is not loaded → false.
fn push_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = match args.first() {
        Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
        Some(Value::String(_)) | Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "PushParticles: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };
    let ax = args
        .get(1)
        .map(|arg| value_to_i32(arg, "PushParticles", "xdir"))
        .transpose()?
        .unwrap_or(0);
    let ay = args
        .get(2)
        .map(|arg| value_to_i32(arg, "PushParticles", "ydir"))
        .transpose()?
        .unwrap_or(0);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("PushParticles requires an active engine context"))?;
        if let Some(name) = &definition {
            if context.particle_def_known(name) == Some(false) {
                return Ok(Value::Bool(false));
            }
        }
        context.register_particle(ParticleCommand::Push {
            definition_id: definition,
            dxdir: ax as f32 / 10.0,
            dydir: ay as f32 / 10.0,
        });
        Ok(Value::Bool(true))
    })
}

fn clear_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut definition: Option<String> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::String(name) if !name.is_empty() => definition = Some(name.clone()),
            Value::String(_) | Value::Nil => definition = None,
            other => {
                return Err(RuntimeError::new(format!(
                    "ClearParticles: expected string or nil for name, got {}",
                    other.type_name()
                )))
            }
        }
        index += 1;
    }

    let mut target_object: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_object = parse_object_reference_argument(arg, "ClearParticles", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "ClearParticles: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };

        // a named def that is not loaded → false (C4Script.cpp:4932)
        if let Some(name) = &definition {
            if context.particle_def_known(name) == Some(false) {
                return Ok(Value::Bool(false));
            }
        }

        let scope = if let Some(target) = target_object {
            if context.get_world_object(target).is_none() {
                return Ok(Value::Bool(false));
            }
            ParticleScope::Object(target)
        } else {
            ParticleScope::Global
        };

        context.register_particle(ParticleCommand::Clear {
            definition_id: definition.clone(),
            scope,
        });
        Ok(Value::Bool(true))
    })
}

fn contained(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Contained expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "Contained", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let to_value = |container: Option<ObjectId>| {
            container.map(object_reference_value).unwrap_or(Value::Nil)
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(to_value(object.container()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(to_value(other.container()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(to_value(object.container()))
    })
}

fn contents(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "Contents expects at most 3 arguments: index, object, include_attached",
        ));
    }

    let index = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "Contents", "index")?,
    };
    if index < 0 {
        return Ok(Value::Nil);
    }

    let target_id =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Contents", "object")?;
    let include_attached = if let Some(value) = args.get(2) {
        value_to_bool(value, "Contents", "include_attached")?
    } else {
        false
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if object.is_present() => object,
            _ => return Ok(Value::Nil),
        };

        let mut entries = Vec::new();
        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                if !include_attached {
                    if let Some(procedure) = child.procedure_name() {
                        if procedure.eq_ignore_ascii_case("attach") {
                            continue;
                        }
                    }
                }
                entries.push(child.id);
            }
        }

        let Some(selected) = entries.get(index as usize) else {
            return Ok(Value::Nil);
        };
        Ok(object_reference_value(*selected))
    })
}

fn contents_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "ContentsCount expects at most 2 arguments: definition, object",
        ));
    }

    let definition = parse_definition_argument(args.first(), "ContentsCount")?;
    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "ContentsCount",
        "object",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(0)),
        };

        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Int(0)),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if object.is_present() => object,
            _ => return Ok(Value::Int(0)),
        };

        let mut count = 0;
        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                if let Some(definition_id) = definition.as_ref() {
                    if child.definition_id() != definition_id {
                        continue;
                    }
                }
                count += 1;
            }
        }

        Ok(Value::Int(count))
    })
}

fn find_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "FindContents expects at least 1 argument: definition",
        ));
    }

    let definition = parse_definition_argument(Some(&args[0]), "FindContents")?;
    let Some(definition) = definition else {
        return Ok(Value::Nil);
    };

    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "FindContents",
        "object",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if object.is_present() => object,
            _ => return Ok(Value::Nil),
        };

        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                if child.definition_id() == definition {
                    return Ok(object_reference_value(child.id));
                }
            }
        }

        Ok(Value::Nil)
    })
}

fn find_other_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "FindOtherContents expects at least 1 argument: definition",
        ));
    }

    let definition = parse_definition_argument(Some(&args[0]), "FindOtherContents")?;
    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "FindOtherContents",
        "object",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if object.is_present() => object,
            _ => return Ok(Value::Nil),
        };

        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                let matches = match definition.as_ref() {
                    Some(definition_id) => child.definition_id() != definition_id,
                    None => true,
                };
                if matches {
                    return Ok(object_reference_value(child.id));
                }
            }
        }

        Ok(Value::Nil)
    })
}

fn get_ocf(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetOCF expects at most 1 argument: target",
        ));
    }

    let target_value = args.first().unwrap_or(&Value::Nil);
    let target_id = parse_object_reference_argument(target_value, "GetOCF", "target")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let ocf_value = |mask: u32| Value::Int(mask as i32);

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(ocf_value(object.ocf()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(ocf_value(other.ocf()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(ocf_value(object.ocf()))
    })
}

fn set_graphics(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetGraphics expects at least 1 argument: graphics name",
        ));
    }

    let graphics_name = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        // Falsy parameters reset to nil before the type check
        // (C4AulExec.cpp:1372): SetGraphics(0) selects the default graphics.
        Value::Int(0) | Value::Bool(false) => None,
        other => {
            return Err(RuntimeError::new(format!(
                "SetGraphics: expected string or nil for graphics name, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let target_id = if let Some(arg) = args.get(index) {
        index += 1;
        parse_object_reference_argument(arg, "SetGraphics", "object")?
    } else {
        None
    };

    let definition = if let Some(arg) = args.get(index) {
        index += 1;
        parse_definition_argument(Some(arg), "SetGraphics")?
    } else {
        None
    };

    let overlay_id = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for overlay id, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        0
    };

    let mode_value = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for overlay mode, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        0
    };

    let action_name = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::String(name) if !name.is_empty() => Some(name.clone()),
            Value::String(_) | Value::Nil => None,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected string or nil for action, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        None
    };

    let blit_mode = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => (*value).max(0) as u32,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for blit mode, got {}",
                    other.type_name()
                )))
            }
        }
    } else {
        0
    };

    let overlay_object = if let Some(arg) = args.get(index) {
        index += 1;
        parse_object_reference_argument(arg, "SetGraphics", "overlay_object")?
    } else {
        None
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetGraphics: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetGraphics requires an active engine context"))?;

        let object_id = if let Some(target) = target_id {
            target
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Bool(false)),
            }
        };

        let mut resolved_definition = definition.clone();
        if overlay_id <= 0 && resolved_definition.is_none() {
            resolved_definition = context
                .get_world_object(object_id)
                .map(|world_object| world_object.definition_id().to_string());
            if resolved_definition.is_none() {
                return Ok(Value::Bool(false));
            }
        }

        if overlay_id <= 0 {
            let definition_id = resolved_definition.expect("resolved definition present");

            if context.definition_metadata(&definition_id).is_none() {
                return Ok(Value::Bool(false));
            }

            let object = match context.object_context_mut() {
                Some(object) => {
                    if object.id != object_id {
                        return Ok(Value::Bool(false));
                    }
                    object
                }
                None => return Ok(Value::Bool(false)),
            };

            let base_graphics = if definition.is_none() && graphics_name.is_none() {
                None
            } else {
                Some(ObjectBaseGraphics {
                    definition: definition_id,
                    graphics_name: graphics_name.clone(),
                    blit_mode,
                })
            };

            let changed = object.set_base_graphics(base_graphics);
            return Ok(Value::Bool(changed));
        }

        let object = match context.object_context_mut() {
            Some(object) => {
                if object.id != object_id {
                    return Ok(Value::Bool(false));
                }
                object
            }
            None => return Ok(Value::Bool(false)),
        };

        if overlay_id < 0 {
            return Ok(Value::Bool(false));
        }

        let mode = if mode_value == 0 {
            GraphicsOverlayMode::Base
        } else {
            match GraphicsOverlayMode::from_script_value(mode_value) {
                Some(mode) => mode,
                None => return Ok(Value::Bool(false)),
            }
        };

        if mode == GraphicsOverlayMode::Object && overlay_object.is_none() {
            let removed = object.remove_graphics_overlay(overlay_id);
            return Ok(Value::Bool(removed));
        }

        if mode != GraphicsOverlayMode::Object && definition.is_none() {
            let removed = object.remove_graphics_overlay(overlay_id);
            return Ok(Value::Bool(removed));
        }

        let overlay = ObjectGraphicsOverlay::new(overlay_id, mode)
            .with_definition(if mode == GraphicsOverlayMode::Object {
                None
            } else {
                definition.clone()
            })
            .with_graphics_name(graphics_name.clone())
            .with_action(action_name)
            .with_blit_mode(blit_mode)
            .with_overlay_object(overlay_object);

        let changed = object.set_graphics_overlay(overlay);
        Ok(Value::Bool(changed))
    })
}

fn parse_draw_transform_components(
    args: &[Value],
    function: &str,
) -> Result<[i32; 6], RuntimeError> {
    if args.len() < 6 {
        return Err(RuntimeError::new(format!(
            "{function} expects at least 6 arguments: a, b, c, d, e, f"
        )));
    }
    Ok([
        value_to_i32(&args[0], function, "a")?,
        value_to_i32(&args[1], function, "b")?,
        value_to_i32(&args[2], function, "c")?,
        value_to_i32(&args[3], function, "d")?,
        value_to_i32(&args[4], function, "e")?,
        value_to_i32(&args[5], function, "f")?,
    ])
}

fn parse_draw_transform_matrix(args: &[Value], function: &str) -> Result<[i32; 9], RuntimeError> {
    if args.len() < 9 {
        return Err(RuntimeError::new(format!(
            "{function} expects at least 9 arguments: a, b, c, d, e, f, g, h, i"
        )));
    }
    Ok([
        value_to_i32(&args[0], function, "a")?,
        value_to_i32(&args[1], function, "b")?,
        value_to_i32(&args[2], function, "c")?,
        value_to_i32(&args[3], function, "d")?,
        value_to_i32(&args[4], function, "e")?,
        value_to_i32(&args[5], function, "f")?,
        value_to_i32(&args[6], function, "g")?,
        value_to_i32(&args[7], function, "h")?,
        value_to_i32(&args[8], function, "i")?,
    ])
}

fn normalize_draw_transform(transform: DrawTransform) -> Option<DrawTransform> {
    if transform.is_identity() {
        None
    } else {
        Some(transform)
    }
}

fn set_obj_draw_transform(args: &[Value]) -> Result<Value, RuntimeError> {
    let components = parse_draw_transform_components(args, "SetObjDrawTransform")?;
    let mut index = 6;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetObjDrawTransform", "object")?;
        index += 1;
    }
    let overlay_id = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "SetObjDrawTransform", "overlay")?;
        index += 1;
        value
    } else {
        0
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjDrawTransform: additional arguments are not supported",
        ));
    }

    let transform = DrawTransform::from_components(
        components[0] as f32 / 1000.0,
        components[4] as f32 / 1000.0,
        components[2] as f32 / 1000.0,
        components[5] as f32 / 1000.0,
    );
    let normalized = normalize_draw_transform(transform);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetObjDrawTransform requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if overlay_id <= 0 {
            object.set_draw_transform(normalized);
            Ok(Value::Bool(true))
        } else {
            let changed = object.set_overlay_transform(overlay_id, normalized);
            Ok(Value::Bool(changed))
        }
    })
}

fn set_obj_draw_transform2(args: &[Value]) -> Result<Value, RuntimeError> {
    let matrix = parse_draw_transform_matrix(args, "SetObjDrawTransform2")?;
    let mut index = 9;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetObjDrawTransform2", "object")?;
        index += 1;
    }
    let overlay_id = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "SetObjDrawTransform2", "overlay")?;
        index += 1;
        value
    } else {
        0
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjDrawTransform2: additional arguments are not supported",
        ));
    }

    let delta = DrawTransform::from_components(
        matrix[0] as f32 / 1000.0,
        matrix[4] as f32 / 1000.0,
        matrix[2] as f32 / 1000.0,
        matrix[5] as f32 / 1000.0,
    );

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetObjDrawTransform2 requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if overlay_id <= 0 {
            let current = object.draw_transform().unwrap_or(DrawTransform::identity());
            let combined = current.combined(delta);
            object.set_draw_transform(normalize_draw_transform(combined));
            Ok(Value::Bool(true))
        } else {
            let existing = match object.overlay_transform(overlay_id) {
                Some(transform) => transform.unwrap_or(DrawTransform::identity()),
                None => return Ok(Value::Bool(false)),
            };
            let combined = existing.combined(delta);
            let changed =
                object.set_overlay_transform(overlay_id, normalize_draw_transform(combined));
            Ok(Value::Bool(changed))
        }
    })
}

fn get_category(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCategory expects at most 2 arguments: target, definition",
        ));
    }

    let target_value = args.first().unwrap_or(&Value::Nil);
    let target_id = parse_object_reference_argument(target_value, "GetCategory", "target")?;
    let definition = parse_definition_argument(args.get(1), "GetCategory")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(definition_id) = definition {
            if let Some(category) = context.definition_category(&definition_id) {
                return Ok(Value::Int(category));
            }
            return Ok(Value::Nil);
        }

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(object.category()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.category()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(object.category()))
    })
}
/// FnCreateContents (C4Script.cpp:1938-1951): create `count` (default 1)
/// objects of `c_id` inside the container, returning the last one. C++
/// routes through pObj->CreateContents -> CreateObject + Enter, with the
/// container's owner.
fn create_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) | Value::C4Id(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::C4Id(_) | Value::Nil | Value::Int(0) => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateContents: expected id for definition, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let target_id = consume_optional_object_reference_argument(
        args,
        &mut index,
        "CreateContents",
        "container",
    )?;
    let count = match args.get(index) {
        // C++: `if (!iCount) ++iCount;`
        Some(arg) => match value_to_i32(arg, "CreateContents", "count")? {
            0 => 1,
            value => value,
        },
        None => 1,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateContents requires an active engine context"))?;

        // CreateContents runs Game.CreateObject too - C4Id2Def failure is
        // a silent nullptr (C4Object::CreateContents, C4Game.cpp:1146).
        if context.world.definition_known(&definition) == Some(false) {
            return Ok(Value::Nil);
        }

        let (container, position, owner) = if let Some(target) = target_id {
            match context.object_context() {
                Some(object) if target == object.id() => {
                    (target, object.effective_position(), object.owner())
                }
                _ => match context.get_world_object(target) {
                    Some(other) => (target, other.position, other.owner),
                    None => return Ok(Value::Nil),
                },
            }
        } else {
            match context.object_context() {
                Some(object) => (object.id(), object.effective_position(), object.owner()),
                None => return Ok(Value::Nil),
            }
        };

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                name: String::new(),
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            });

        // "controller will automatically be set upon entrance"
        // (FnCreateContents, C4Script.cpp:1944): the preview mirrors the
        // Enter transfer (C4Object.cpp:1579-1582) — non-living contents
        // assume the container's controller, the rest keep the Init owner.
        let preview_controller = if metadata.category & crate::CATEGORY_LIVING != 0 {
            owner
        } else {
            context
                .object_context()
                .filter(|object| object.id() == container)
                .map(ObjectScopeContext::controller)
                .or_else(|| {
                    context
                        .get_world_object(container)
                        .map(|object| object.controller())
                })
                .unwrap_or(owner)
        };
        let mut last = Value::Nil;
        for _ in 0..count {
            let id = context.allocate_object_id();
            let spawn = SpawnConfig::new(definition.clone())
                .with_position(position)
                .with_owner(owner)
                .with_category(metadata.category)
                .with_container(container)
                .with_id(id);
            let preview_ocf = ocf::compute(
                metadata.ocf_base,
                metadata.crew_member,
                true,
                ObjectStatus::Normal,
                false,
                FULL_CON,
            );
            let preview = HostWorldObject::with_category(
                id,
                definition.clone(),
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                owner,
                metadata.category,
                0,
                FULL_CON,
                0,
                position,
                Vector2::ZERO,
                0,
                Vec::new(),
                0,
                0,
                0,
                None,
                None,
            )
            .with_ocf(preview_ocf)
            .with_full_state(Rc::new({
                let mut state = crate::preview_spawn_state(
                    position,
                    owner,
                    preview_controller,
                    metadata.category,
                    FULL_CON,
                    metadata.vertices.clone(),
                );
                state.container = Some(container);
                state
            }));
            context.register_spawn(spawn, preview);
            last = object_reference_value(id);
        }
        Ok(last)
    })
}

/// FnGetActMapVal (C4Script.cpp:4216-4241): one entry of one action in a
/// definition's ActMap, addressed by its serialization name
/// (C4ActionDef::CompileFunc, C4Def.cpp). Unknown definition, action or
/// entry -> nil. C4ActionDef compile defaults: Length 1, Delay 0, strings "".
fn get_act_map_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) => name.clone(),
        Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "GetActMapVal: expected string for entry, got {}",
                other.type_name()
            )))
        }
    };
    let action = match args.get(1).unwrap_or(&Value::Nil) {
        Value::String(name) => name.clone(),
        Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "GetActMapVal: expected string for action, got {}",
                other.type_name()
            )))
        }
    };
    let definition = match args.get(2).unwrap_or(&Value::Nil) {
        Value::String(name) | Value::C4Id(name) if !name.is_empty() => Some(name.clone()),
        _ => None,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        // `idDef` defaults to the executing definition (cthr->Def).
        let library = match definition {
            Some(id) => match context.definition_metadata(&id) {
                Some(metadata) => metadata.action_library.clone(),
                None => return Ok(Value::Nil),
            },
            None => match context.object_context() {
                Some(object) => object.action_library.clone(),
                None => return Ok(Value::Nil),
            },
        };
        let Some(spec) = library.specs().get(&action) else {
            return Ok(Value::Nil);
        };

        Ok(match entry.as_str() {
            "Name" => Value::String(action.clone()),
            "Procedure" => Value::String(spec.procedure.clone().unwrap_or_default()),
            "Length" => Value::Int(spec.length.unwrap_or(1) as i32),
            "Delay" => Value::Int(spec.delay.unwrap_or(0) as i32),
            "Attach" => Value::Int(spec.attach as i32),
            "NextAction" => Value::String(spec.next.clone().unwrap_or_default()),
            "StartCall" => Value::String(spec.start_call.clone().unwrap_or_default()),
            "EndCall" => Value::String(spec.end_call.clone().unwrap_or_default()),
            "AbortCall" => Value::String(spec.abort_call.clone().unwrap_or_default()),
            "PhaseCall" => Value::String(spec.phase_call.clone().unwrap_or_default()),
            "NoOtherAction" => Value::Int(i32::from(spec.no_other_action)),
            "DigFree" => Value::Int(spec.dig_free.unwrap_or(0)),
            _ => Value::Nil,
        })
    })
}

/// FnGetObjectVal (C4Script.cpp:4184-4195): reflect one entry of the
/// object's serialization (C4Object::CompileFunc; the Shape is compiled
/// inline, so "Width"/"Height" are the shape rect, C4Shape.cpp:496-516).
/// Entries outside our model -> nil.
fn get_object_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) => name.clone(),
        Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "GetObjectVal: expected string for entry, got {}",
                other.type_name()
            )))
        }
    };
    // args[1] is the section name; every entry name we serve is unique.
    let mut index = 2;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetObjectVal", "target")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let self_id = context.object_context().map(|object| object.id());
        let resolved_target = target_id.or(self_id);
        let Some(target) = resolved_target else {
            return Ok(Value::Nil);
        };

        if Some(target) == self_id {
            if let Some(object) = context.object_context() {
                match entry.as_str() {
                    "Owner" => return Ok(Value::Int(object.owner())),
                    "Category" => return Ok(Value::Int(object.category())),
                    "Energy" => return Ok(Value::Int(object.current_energy)),
                    "Damage" => return Ok(Value::Int(object.current_damage)),
                    _ => {}
                }
            }
        }

        let Some(world_object) = context.get_world_object(target) else {
            return Ok(Value::Nil);
        };
        Ok(match entry.as_str() {
            "Owner" => Value::Int(world_object.owner),
            "Category" => Value::Int(world_object.category),
            "Energy" => Value::Int(world_object.energy),
            "Damage" => Value::Int(world_object.damage),
            "Width" | "Height" => context
                .definition_metadata(world_object.definition_id())
                .and_then(|metadata| metadata.shape)
                .map(|shape| {
                    Value::Int(if entry == "Width" {
                        shape.width
                    } else {
                        shape.height
                    })
                })
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        })
    })
}

/// FnSetEntrance (C4Script.cpp:690-695): toggle the object's EntranceStatus.
fn set_entrance(args: &[Value]) -> Result<Value, RuntimeError> {
    let enabled = args.first().unwrap_or(&Value::Nil).as_bool();
    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetEntrance", "target")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetEntrance requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        object.pending_update.entrance_status = Some(enabled);
        Ok(Value::Bool(true))
    })
}

/// FnSetColorDw (C4Script.cpp:3661-3668): set the object's 32-bit color.
fn set_color_dw(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetColorDw", "value")?;
    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetColorDw", "target")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetColorDw requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        object.pending_update.color = Some(value as u32);
        Ok(Value::Bool(true))
    })
}

/// FnSetShape (C4Script.cpp:5182-5196): overwrite the object's shape rect.
fn set_shape(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetShape", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetShape", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetShape", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "SetShape", "hgt")?;
    let mut index = 4;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetShape", "target")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetShape requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }
        object.pending_update.shape_override = Some(DefinitionRect::new(x, y, width, height));
        Ok(Value::Bool(true))
    })
}

/// FnSetVertex (C4Script.cpp:1237-1271): set one vertex attribute (VTX_X=0,
/// VTX_Y=1, VTX_CNAT=2, VTX_Friction=3); unknown attributes fall back to
/// VtxY like the old-style C++ behaviour. Own-vertex mode offsets the index
/// by C4D_VertexCpyPos = C4D_MaxVertex/2 = 15 (C4Shape.h:27).
fn set_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    const MAX_VERTEX: usize = 30;
    let index_arg = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetVertex", "index")?;
    let kind = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetVertex", "attribute")?;
    let value = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetVertex", "value")?;
    let mut arg_index = 3;
    let target_id =
        consume_optional_object_reference_argument(args, &mut arg_index, "SetVertex", "target")?;
    let own_vertex_mode = match args.get(arg_index) {
        Some(arg) => value_to_i32(arg, "SetVertex", "own vertex mode")?,
        None => 0,
    };

    let mut vertex_index = index_arg;
    if own_vertex_mode != 0 {
        vertex_index += 15;
    }
    let Ok(vertex_index) = usize::try_from(vertex_index) else {
        return Ok(Value::Bool(false));
    };
    if vertex_index >= MAX_VERTEX {
        return Ok(Value::Bool(false));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetVertex requires an active engine context"))?;
        let active = context.object_context().map(|object| object.id());
        let write = |vertices_base: &[ObjectVertex], pending: &mut Option<Vec<ObjectVertex>>| {
            let mut vertices = pending.clone().unwrap_or_else(|| vertices_base.to_vec());
            if vertices.len() <= vertex_index {
                vertices.resize(vertex_index + 1, ObjectVertex::default());
            }
            match kind {
                0 => vertices[vertex_index].x = value,
                2 => vertices[vertex_index].cnat = value as u32,
                3 => vertices[vertex_index].friction = value,
                // VTX_Y and the old-style fallback for any other attribute.
                _ => vertices[vertex_index].y = value,
            }
            *pending = Some(vertices);
        };

        // FnSetVertex works on ANY object (`if (!pObj) pObj = cthr->Obj`,
        // C4Script.cpp) — the Gatling aims its crosshair with
        // SetVertex(0, .., pCrosshair). Foreign writes stage into the
        // target's nested scope like every other cross-object fold.
        match target_id {
            Some(target) if Some(target) != active => {
                if !context.nested_objects.contains_key(&target) {
                    let Some(world_object) = context.get_world_object(target) else {
                        return Ok(Value::Bool(false));
                    };
                    let Some((scope, local_vars)) = context.nested_scope_for(&world_object) else {
                        return Ok(Value::Bool(false));
                    };
                    context
                        .nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
                if !context.nested_order.contains(&target) {
                    context.nested_order.push(target);
                }
                let state = context
                    .nested_objects
                    .get_mut(&target)
                    .expect("scope just ensured");
                let base = state.scope.vertices.clone();
                write(&base, &mut state.scope.pending_update.vertices);
                Ok(Value::Bool(true))
            }
            _ => {
                let object = match context.object_context_mut() {
                    Some(object) => object,
                    None => return Ok(Value::Bool(false)),
                };
                let base = object.vertices.clone();
                write(&base, &mut object.pending_update.vertices);
                Ok(Value::Bool(true))
            }
        }
    })
}

/// FindObject container sentinels (C4Object.h:83-84): `NoContainer()` = 124,
/// `AnyContainer()` = 123 (FnNoContainer/FnAnyContainer,
/// C4Script.cpp:6731-6732).
/// FnInsertMaterial (C4Script.cpp:2207-2211): insert one material pixel
/// at caller-relative coordinates. The vx/vy PXS velocity is accepted and
/// dropped (the rust landscape settles inserted material directly;
/// PORT_STATUS: no PXS particle sim).
fn insert_material(args: &[Value]) -> Result<Value, RuntimeError> {
    let material = value_to_i32(args.first().unwrap_or(&Value::Nil), "InsertMaterial", "mat")?;
    let x = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "InsertMaterial", "x")?;
    let y = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "InsertMaterial", "y")?;
    // FnInsertMaterial (C4Script.cpp:2207-2211): vx/vy ride into
    // C4Landscape::InsertMaterial (FIXED10 there).
    let vx = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "InsertMaterial", "vx")?;
    let vy = value_to_i32(args.get(4).unwrap_or(&Value::Nil), "InsertMaterial", "vy")?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };
        let mut position = Vector2::new(x, y);
        if let Some(object) = context.object_context() {
            let base = object.current_position;
            position = Vector2::new(base.x + x, base.y + y);
        }
        context.register_landscape_operation(LandscapeOperation::InsertMaterial {
            material,
            position,
            velocity: Vector2::new(vx, vy),
        });
        Ok(Value::Bool(true))
    })
}

/// FnExtractMaterialAmount (C4Script.cpp:2264-2273): extract up to
/// `amount` pixels while `GBackMat(x,y) == mat`, each through
/// ExtractMaterial (FindMatTop + clear). The count is computed by an
/// overlay simulation on the read view and the mutation staged as an
/// operation applied on the same state.
fn extract_material_amount(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "ExtractMaterialAmount", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "ExtractMaterialAmount", "y")?;
    let material = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "mat",
    )?;
    let amount = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "amount",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Int(0)),
        };
        let mut position = Vector2::new(x, y);
        if let Some(object) = context.object_context() {
            let base = object.current_position;
            position = Vector2::new(base.x + x, base.y + y);
        }
        let Some(material_id) = usize::try_from(material).ok().and_then(crate::material::MaterialId::new)
        else {
            return Ok(Value::Int(0));
        };
        let Some(materials) = context.world.materials() else {
            return Ok(Value::Int(0));
        };
        let extracted = context
            .world
            .landscape_ref()
            .map(|landscape| {
                landscape.simulate_extract_material_amount(
                    materials,
                    position.x,
                    position.y,
                    material_id,
                    amount,
                )
            })
            .unwrap_or(0);
        if extracted > 0 {
            context.register_landscape_operation(LandscapeOperation::ExtractMaterialAmount {
                material,
                position,
                amount: extracted,
            });
        }
        Ok(Value::Int(extracted))
    })
}

/// FnOnFire (C4Script.cpp:1870-1877): burning when the fire flag is set or/// FnOnFire (C4Script.cpp:1870-1877): burning when the fire flag is set or
/// any *Fire* effect (C4Fx_AnyFire) sits on the object; nil without one.
fn on_fire(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "OnFire", "target")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        if let Some(object) = context.object_context() {
            if target_id.is_none() || target_id == Some(object.id()) {
                let burning = object
                    .effects
                    .snapshot()
                    .iter()
                    .any(|effect| effect.name.contains("Fire"));
                return Ok(Value::Bool(burning));
            }
        }
        let Some(target) = target_id else {
            return Ok(Value::Nil);
        };
        match context.get_world_object(target) {
            Some(other) => Ok(Value::Bool(other.full_state().is_some_and(|state| {
                state
                    .effects
                    .iter()
                    .any(|effect| effect.name.contains("Fire"))
            }))),
            None => Ok(Value::Nil),
        }
    })
}

fn no_container(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Int(124))
}

fn any_container(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Int(123))
}

/// FnActIdle (C4Script.cpp:1831-1836): true when the object has no action
/// (C++ Act == ActIdle; our engine stores that as an empty or "Idle" name),
/// nil without an object.
fn act_idle(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "ActIdle", "target")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let action_name = if let Some(target) = target_id {
            match context.object_context() {
                Some(object) if target == object.id() => {
                    Some(object.effective_action_name().to_string())
                }
                _ => context
                    .get_world_object(target)
                    .map(|other| other.action_name.clone()),
            }
        } else {
            context
                .object_context()
                .map(|object| object.effective_action_name().to_string())
        };

        Ok(action_name
            .map(|name| Value::Bool(name.is_empty() || name == "Idle"))
            .unwrap_or(Value::Nil))
    })
}

fn set_category(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetCategory expects at least 1 argument: category",
        ));
    }

    let category = value_to_i32(&args[0], "SetCategory", "category")?;

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetCategory", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetCategory: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetCategory requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_category(category);
        Ok(Value::Bool(true))
    })
}

fn set_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetOwner expects at least 1 argument: owner",
        ));
    }

    let owner = match &args[0] {
        Value::Int(value) => *value,
        other => {
            return Err(RuntimeError::new(format!(
                "SetOwner: expected int for owner, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetOwner", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetOwner: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetOwner requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_owner(owner);
        Ok(Value::Bool(true))
    })
}

fn set_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetAlive expects at least 1 argument: alive",
        ));
    }

    let alive = match &args[0] {
        Value::Bool(flag) => *flag,
        Value::Int(value) => *value != 0,
        Value::Nil => false,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAlive: expected bool, int, or nil for alive, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetAlive", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetAlive: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetAlive requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_alive(alive);
        Ok(Value::Bool(true))
    })
}

fn get_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetOwner expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetOwner", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(OWNER_NONE)),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.owner()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.owner()));
            }
            return Ok(Value::Int(OWNER_NONE));
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Int(OWNER_NONE)),
        };

        Ok(Value::Int(object.owner()))
    })
}

/// FnGetController (C4Script.cpp:1316-1320): the object's Controller,
/// NO_OWNER without an object.
fn get_controller(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetController expects at most 1 argument: target",
        ));
    }

    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetController", "target"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(OWNER_NONE)),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.controller()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.controller()));
            }
            return Ok(Value::Int(OWNER_NONE));
        }

        let controller = context
            .object_context()
            .map(|object| object.controller())
            .unwrap_or(OWNER_NONE);
        Ok(Value::Int(controller))
    })
}

/// FnIncinerateLandscape (C4Script.cpp:253-261) -> C4Landscape::Incinerate
/// (C4Landscape.cpp:1430-1441): caller-relative point; inflammable
/// material lights one FLAM unless another already burns in the
/// (x-4, y-1, 8, 20) rect (C4Game::FindObject center-in-rect range check,
/// C4Game.cpp: `Inside(cObj->x - iX, 0, iWdt-1)`). C++ creates the FLAM
/// mid-call (Game.CreateObject); the port registers a pending spawn, so
/// its creation callbacks run at materialization within the same frame.
fn incinerate_landscape(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "IncinerateLandscape expects at most 2 arguments: x, y",
        ));
    }
    let mut x = parse_optional_i32(args.first(), "IncinerateLandscape", "x")?.unwrap_or(0);
    let mut y = parse_optional_i32(args.get(1), "IncinerateLandscape", "y")?.unwrap_or(0);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("IncinerateLandscape requires an active engine context")
        })?;

        // Local calls offset by the object position (C4Script.cpp:255-259).
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            x = x.saturating_add(position.x);
            y = y.saturating_add(position.y);
        }

        let inflammable = match (context.landscape_ref(), context.world.materials()) {
            (Some(landscape), Some(materials)) => landscape.can_incinerate(x, y, materials),
            _ => false,
        };
        if !inflammable {
            return Ok(Value::Bool(false));
        }

        let Some(metadata) = context.definition_metadata(crate::FIRE_DEFINITION_ID).cloned() else {
            // Unknown FLAM def: Game.CreateObject returns nullptr.
            return Ok(Value::Bool(false));
        };

        // "Not too much FLAMs" (C4Landscape.cpp:1436-1437) — pending
        // same-call spawns count like the live objects C++ would find.
        let left = x.saturating_sub(4);
        let right = left.saturating_add(8);
        let top = y.saturating_sub(1);
        let bottom = top.saturating_add(20);
        let burning = context.world_object_ids().into_iter().any(|id| {
            context
                .get_world_object(id)
                .filter(|object| object.definition_id() == crate::FIRE_DEFINITION_ID)
                .filter(|object| object.status().is_active())
                .map(|object| {
                    let pos = object.position;
                    pos.x >= left && pos.x < right && pos.y >= top && pos.y < bottom
                })
                .unwrap_or(false)
        });
        if burning {
            return Ok(Value::Bool(false));
        }

        let id = context.allocate_object_id();
        let spawn = SpawnConfig::new(crate::FIRE_DEFINITION_ID)
            .with_position(Vector2::new(x, y))
            .with_category(metadata.category)
            .with_id(id);
        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            true,
            ObjectStatus::Normal,
            false,
            FULL_CON,
        );
        let preview = HostWorldObject::with_category(
            id,
            crate::FIRE_DEFINITION_ID,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            metadata.category,
            0,
            FULL_CON,
            0,
            Vector2::new(x, y),
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
            None,
        )
        .with_ocf(preview_ocf)
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::new(x, y),
            OWNER_NONE,
            OWNER_NONE,
            metadata.category,
            FULL_CON,
            metadata.vertices.clone(),
        )));
        context.register_spawn(spawn, preview);
        Ok(Value::Bool(true))
    })
}

/// FnDistance (C4Script.cpp:3316-3319) -> Distance (C4Math.cpp:22-31):
/// integer euclidean distance; the float sqrt is post-adjusted to the
/// exact integer floor (++/-- until dist^2 brackets d2). Negative d2
/// (int64 overflow in C++) returns -1.
fn distance(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new(
            "Distance expects at most 4 arguments: x1, y1, x2, y2",
        ));
    }
    let x1 = parse_optional_i32(args.first(), "Distance", "x1")?.unwrap_or(0);
    let y1 = parse_optional_i32(args.get(1), "Distance", "y1")?.unwrap_or(0);
    let x2 = parse_optional_i32(args.get(2), "Distance", "x2")?.unwrap_or(0);
    let y2 = parse_optional_i32(args.get(3), "Distance", "y2")?.unwrap_or(0);

    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if d2 < 0 {
        return Ok(Value::Int(-1));
    }
    let mut dist = (d2 as f64).sqrt() as i64;
    if dist.wrapping_mul(dist) < d2 {
        dist += 1;
    }
    if dist.wrapping_mul(dist) > d2 {
        dist -= 1;
    }
    Ok(Value::Int(dist as i32))
}

/// FnSetViewOffset (C4Script.cpp:5676-5687): ValidPlr gate; without a
/// viewport (the headless/dedicated path) the write is skipped and the
/// call still succeeds ("sync safety").
fn set_view_offset(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetViewOffset expects at most 3 arguments: player, x, y",
        ));
    }
    let player = parse_optional_i32(args.first(), "SetViewOffset", "player")?.unwrap_or(0);
    // x/y validate like every int parameter even though no viewport
    // exists to store them.
    let _ = parse_optional_i32(args.get(1), "SetViewOffset", "x")?;
    let _ = parse_optional_i32(args.get(2), "SetViewOffset", "y")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let valid = borrow
            .as_ref()
            .map(|context| context.player_state(player).is_some())
            .unwrap_or(false);
        Ok(Value::Bool(valid))
    })
}

/// FnGetObjectLayer (C4Script.cpp:5160-5166): the object's pLayer — nil
/// unless a layer was assigned (Objects.txt `Layer=`; SetObjectLayer is
/// unported). Layers cannot change mid-call, so the world snapshot is
/// authoritative for the context object too.
fn get_object_layer(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetObjectLayer expects at most 1 argument: target",
        ));
    }

    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetObjectLayer", "target"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let layer = target
            .and_then(|target| context.get_world_object(target))
            .and_then(|object| object.full_state().and_then(|state| state.layer));
        Ok(layer.map(object_reference_value).unwrap_or(Value::Nil))
    })
}

/// FnSetController (C4Script.cpp:1322-1331): NO_OWNER always passes, any
/// other value must be a valid player; foreign targets are written via the
/// nested-call seam like RemoveObject.
fn set_controller(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "SetController expects 1 or 2 arguments: controller and optional target",
        ));
    }

    let new_controller = value_to_i32(&args[0], "SetController", "controller")?;
    let target_id = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetController", "target"))
        .transpose()?
        .flatten();

    let (valid_player, active) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref();
        (
            context
                .map(|context| context.player_state(new_controller).is_some())
                .unwrap_or(false),
            context
                .and_then(|context| context.object_context().map(|object| object.id())),
        )
    });
    // validate player (C4Script.cpp:1325)
    if new_controller != OWNER_NONE && !valid_player {
        return Ok(Value::Bool(false));
    }

    if let Some(target) = target_id {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetController",
                &[Value::Int(new_controller)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        object.set_controller(new_controller);
        Ok(Value::Bool(true))
    })
}

fn get_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetAlive expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetAlive", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Bool(object.alive()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Bool(other.alive()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Bool(object.alive()))
    })
}

fn remove_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "RemoveObject expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "RemoveObject", "target")?;
    }

    // FnRemoveObject (C4Script.cpp:455-460): no argument means the calling
    // object, and ANY object may be removed — a foreign target's removal
    // lands in its own scope (GoldRush's DoInitialize culls placed editor
    // leftovers via RemoveObject(FindObject(_ETG))).
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target_id {
        if Some(target) != active {
            if let Some(result) = call_world_object_function(target, "RemoveObject", &[]) {
                return result;
            }
            // The nested seam cannot reach spawns of the SAME call (no
            // full state yet) — cancel the pending spawn directly. Its
            // number stays consumed, exactly like C++ where the object
            // existed and died (the GoldRush TRPR Recruitment temp,
            // Trapper.c4d/Script.c:19-25).
            return HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                let Some(context) = borrow.as_mut() else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(context.cancel_pending_spawn(target)))
            });
        }
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        object.mark_destroy();
        Ok(Value::Bool(true))
    })
}

fn with_context_mut<R>(
    scope: EffectScope,
    func: impl FnOnce(&mut EffectScopeContext) -> R,
) -> Result<R, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("effect host functions require an active engine context")
        })?;
        let stack = context.scope_mut(scope)?;
        Ok(func(stack))
    })
}

fn set_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetObjectStatus expects at least 1 argument: status",
        ));
    }

    let status_value = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetObjectStatus: expected int or nil for status, got {}",
                other.type_name()
            )))
        }
    };

    let status = match ObjectStatus::from_script_value(status_value) {
        Some(status) => status,
        None => return Ok(Value::Bool(false)),
    };

    if matches!(status, ObjectStatus::Deleted) {
        return Ok(Value::Bool(false));
    }

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetObjectStatus", "target")?;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Bool(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "SetObjectStatus: expected bool or nil for clear pointers, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjectStatus: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetObjectStatus requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_status(status);
        Ok(Value::Bool(true))
    })
}

fn get_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetObjectStatus expects at most 1 argument",
        ));
    }

    let target_id = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetObjectStatus",
        "target",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.status().to_script_value()))
    })
}

fn snapshot_effects_from_context(scope: EffectScope) -> Option<Vec<EffectState>> {
    HOST_CONTEXT.with(|cell| cell.borrow().as_ref().and_then(|ctx| ctx.snapshot(scope)))
}

fn determine_scope_from_state(value: &Value) -> Result<EffectScope, RuntimeError> {
    match value {
        Value::Object(_) | Value::Proplist(_) => Ok(EffectScope::Object),
        Value::Nil => Ok(EffectScope::Global),
        Value::Int(id) if *id == 0 => Ok(EffectScope::Global),
        other => Err(RuntimeError::new(format!(
            "effect host functions expected object, proplist, nil, or 0 for state, got {}",
            other.type_name()
        ))),
    }
}

fn extract_effects_from_state(state: &Value) -> Result<Vec<EffectState>, RuntimeError> {
    let map = match state {
        Value::Proplist(map) => map,
        Value::Object(_) => return Ok(Vec::new()),
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected object, proplist, or nil for state, got {}",
                other.type_name()
            )))
        }
    };

    let effects_value = map.get("effects").unwrap_or(&Value::Nil);
    match effects_value {
        Value::Nil => Ok(Vec::new()),
        Value::Array(entries) => {
            let mut effects = Vec::new();
            for entry in entries {
                let props = match entry {
                    Value::Proplist(props) => props,
                    _ => continue,
                };

                let name = match props.get("name") {
                    Some(Value::String(name)) if !name.is_empty() => name.clone(),
                    _ => continue,
                };

                let priority = match props.get("priority") {
                    Some(Value::Int(value)) => *value,
                    _ => 0,
                };

                let interval = match props.get("interval") {
                    Some(Value::Int(value)) if *value > 0 => *value,
                    _ => 1,
                };

                let timer = match props.get("timer") {
                    Some(Value::Int(value)) if *value >= 0 => *value,
                    _ => 0,
                };

                let command_target = match props.get("command_target") {
                    Some(Value::Int(value)) => Some(*value),
                    _ => None,
                };
                let command_id = match props.get("command_target_id") {
                    Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                    _ => None,
                };

                let vars = match props.get("vars") {
                    Some(Value::Array(entries)) => {
                        entries.iter().map(value_to_effect_var).collect()
                    }
                    _ => Vec::new(),
                };

                let mut effect = EffectState::new(name)
                    .with_priority(priority)
                    .with_interval(interval)
                    .with_timer(timer)
                    .with_command_target(command_target)
                    .with_command_id(command_id);
                if !vars.is_empty() {
                    effect = effect.with_vars(vars);
                }
                // Fixture proplists usually carry no allocated iNumber; the
                // list position + 1 stands in so by-number lookups
                // (GetEffect without name, EffectVar, EffectCall) stay
                // usable on snapshot state.
                effect.number = match props.get("number") {
                    Some(Value::Int(value)) if *value > 0 => *value,
                    _ => i32::try_from(effects.len() + 1).unwrap_or(i32::MAX),
                };
                effects.push(effect);
            }
            Ok(effects)
        }
        other => Err(RuntimeError::new(format!(
            "GetEffect: state.effects must be an array, got {}",
            other.type_name()
        ))),
    }
}

fn build_effect_value(effect: &EffectState) -> Value {
    let mut map = HashMap::with_capacity(6);
    map.insert("number".into(), Value::Int(effect.number));
    map.insert("name".into(), Value::String(effect.name.clone()));
    map.insert("priority".into(), Value::Int(effect.priority));
    map.insert("interval".into(), Value::Int(effect.interval));
    map.insert("timer".into(), Value::Int(effect.timer));
    if let Some(target) = effect.command_target {
        map.insert("command_target".into(), Value::Int(target));
    }
    if let Some(id) = &effect.command_id {
        map.insert("command_target_id".into(), Value::String(id.clone()));
    }
    if !effect.vars().is_empty() {
        let vars = effect.vars().iter().map(effect_var_to_value).collect();
        map.insert("vars".into(), Value::Array(vars));
    }
    Value::Proplist(map)
}

fn value_to_effect_var(value: &Value) -> EffectVarValue {
    match value {
        Value::Int(value) => EffectVarValue::Int(*value),
        Value::Bool(value) => EffectVarValue::Bool(*value),
        Value::String(value) => EffectVarValue::String(value.clone()),
        Value::C4Id(id) => EffectVarValue::String(id.clone()),
        Value::Object(id) => EffectVarValue::Object(*id),
        Value::Array(entries) => {
            let vars = entries.iter().map(value_to_effect_var).collect();
            EffectVarValue::Array(vars)
        }
        Value::Proplist(map) => {
            let mut entries = BTreeMap::new();
            for (key, value) in map {
                entries.insert(key.clone(), value_to_effect_var(value));
            }
            EffectVarValue::Proplist(entries)
        }
        Value::Nil => EffectVarValue::Nil,
    }
}

fn effect_var_to_value(value: &EffectVarValue) -> Value {
    match value {
        EffectVarValue::Int(value) => Value::Int(*value),
        EffectVarValue::Bool(value) => Value::Bool(*value),
        EffectVarValue::String(value) => Value::String(value.clone()),
        EffectVarValue::Object(id) => Value::Object(*id),
        EffectVarValue::Array(entries) => {
            let vars = entries.iter().map(effect_var_to_value).collect();
            Value::Array(vars)
        }
        EffectVarValue::Proplist(map) => {
            let mut entries = HashMap::with_capacity(map.len());
            for (key, value) in map {
                entries.insert(key.clone(), effect_var_to_value(value));
            }
            Value::Proplist(entries)
        }
        EffectVarValue::Nil => Value::Nil,
    }
}

fn parse_command_target(value: &Value) -> Result<Option<i32>, RuntimeError> {
    match value {
        Value::Object(_) => Ok(object_id_from_value(value).map(|id| truncate_to_i32(id.as_u64()))),
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) => Ok(Some(*id)),
            _ => Err(RuntimeError::new(
                "AddEffect: command target proplist must contain int `id`",
            )),
        },
        Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected object, proplist, nil, or 0 for command target, got {}",
            other.type_name()
        ))),
    }
}

fn parse_command_target_id(value: &Value) -> Result<Option<String>, RuntimeError> {
    match value {
        Value::String(id) if !id.is_empty() => Ok(Some(id.clone())),
        Value::String(_) | Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected string or nil for command target id, got {}",
            other.type_name()
        ))),
    }
}

fn parse_timer_from_int(value: i32) -> Result<i32, RuntimeError> {
    if value < 0 {
        Err(RuntimeError::new(
            "AddEffect: timer must be >= 0 when provided",
        ))
    } else {
        Ok(value)
    }
}

pub(crate) fn enter_audio_context(audio: AudioRegistry) -> AudioContextGuard {
    AUDIO_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested audio contexts are not supported"
        );
        *cell.borrow_mut() = Some(audio);
    });
    AudioContextGuard { consumed: false }
}

pub(crate) struct AudioContextGuard {
    consumed: bool,
}

impl AudioContextGuard {
    pub fn finish(mut self) -> AudioRegistry {
        self.consumed = true;
        AUDIO_CONTEXT
            .with(|cell| cell.borrow_mut().take())
            .unwrap_or_default()
    }
}

impl Drop for AudioContextGuard {
    fn drop(&mut self) {
        if !self.consumed {
            let _ = AUDIO_CONTEXT.with(|cell| cell.borrow_mut().take());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AudioInstanceKey {
    name: String,
    target: Option<ObjectId>,
}

#[derive(Debug, Clone)]
struct AudioInstance {
    volume: u8,
    custom_falloff: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AudioRegistry {
    looping: HashMap<AudioInstanceKey, AudioInstance>,
    events: Vec<AudioCommand>,
}

#[derive(Debug, Clone)]
pub(crate) struct AudioOutcome {
    pub state: AudioRegistry,
    pub events: Vec<AudioCommand>,
}

impl AudioRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_looping(&self, name: &str, target: Option<ObjectId>) -> bool {
        let key = AudioInstanceKey {
            name: normalize_sound_name(name),
            target,
        };
        self.looping.contains_key(&key)
    }

    pub fn play_sound(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: u8,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
    ) {
        if looped && !multiple {
            let key = AudioInstanceKey {
                name: normalize_sound_name(name),
                target,
            };
            if self.looping.contains_key(&key) {
                return;
            }
            self.looping.insert(
                key,
                AudioInstance {
                    volume,
                    custom_falloff,
                },
            );
        } else if looped {
            let key = AudioInstanceKey {
                name: normalize_sound_name(name),
                target,
            };
            self.looping.insert(
                key,
                AudioInstance {
                    volume,
                    custom_falloff,
                },
            );
        }

        self.events.push(AudioCommand::PlaySound {
            name: name.to_string(),
            target,
            volume,
            looped,
            custom_falloff,
        });
    }

    pub fn stop_sound(&mut self, name: &str, target: Option<ObjectId>) {
        let key = AudioInstanceKey {
            name: normalize_sound_name(name),
            target,
        };
        self.looping.remove(&key);
        self.events.push(AudioCommand::StopSound {
            name: name.to_string(),
            target,
        });
    }

    pub fn set_volume(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: u8,
        custom_falloff: Option<i32>,
    ) -> bool {
        let key = AudioInstanceKey {
            name: normalize_sound_name(name),
            target,
        };
        let existed = if let Some(instance) = self.looping.get_mut(&key) {
            if let Some(falloff) = custom_falloff {
                instance.custom_falloff = Some(falloff);
            }
            instance.volume = volume;
            true
        } else {
            self.looping.insert(
                key,
                AudioInstance {
                    volume,
                    custom_falloff,
                },
            );
            false
        };
        if existed {
            self.events.push(AudioCommand::SetSoundVolume {
                name: name.to_string(),
                target,
                volume,
            });
        }
        existed
    }

    pub fn take_events(&mut self) -> Vec<AudioCommand> {
        mem::take(&mut self.events)
    }
}

impl Default for AudioOutcome {
    fn default() -> Self {
        Self {
            state: AudioRegistry::new(),
            events: Vec::new(),
        }
    }
}

fn normalize_sound_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// A completed nested call's scope plus its VM-final local variables, kept so
/// a later nested call on the same object resumes from the accumulated state
/// (C++ mutates live state, so repeat calls see earlier changes).
struct NestedScopeState {
    scope: ObjectScopeContext,
    local_vars: HashMap<String, Value>,
}

/// Where a nested call's scope came from (and must return to).
enum NestedScopeOrigin {
    /// `dormant_scopes[index]` — the target is an in-flight outer call.
    Dormant(usize),
    /// The completed-call map (or a fresh snapshot scope).
    Completed,
}

/// Phase-1 result of [`EffectHostContext::prepare_nested_call`]: everything
/// the caller needs to run the nested VM after releasing the borrow.
/// `origin: None` means the target was already the active scope.
struct NestedCallPrep {
    script: Arc<ScriptEngine>,
    local_vars: HashMap<String, Value>,
    origin: Option<NestedScopeOrigin>,
}

/// Runs `function` on `target`'s definition script from inside a running VM
/// call — the host→VM reentrancy seam (C4FindObjectFunc::Check,
/// C4FindObject.cpp:653-662: `pCallFunc->Exec(pObj, Pars, true)`): the
/// target object is the call context (`this`), never a parameter. Returns
/// `None` when the function is not visible to the target (C++ fails the
/// check silently) and `Some(Err(_))` for runtime errors (`fPassErrors=true`
/// — the caller rethrows, aborting the calling script).
pub(crate) fn call_world_object_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, true, true, None)
}

/// `obj->ID::Func(...)` (AB_CALLNS, C4AulParse.cpp:3160-3245): runs the
/// NAMED def's function with the target object as context — the target's
/// own same-name function is bypassed. Script functions only (GetSFunc).
pub(crate) fn call_world_object_function_in_scope(
    target: ObjectId,
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, false, Some(script))
}

/// Like [`call_world_object_function`], but resolves SCRIPT functions only —
/// the owner-scoped `GetSFunc` lookup the Call family uses (C4Aul.cpp:
/// 295-298, 562-576): engine (host) functions are never found, unlike
/// Find_Func's `FindSameNameFunc` engine fallback.
pub(crate) fn call_world_object_script_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, true, None)
}

/// Object-call resolution (`C4Object::Call` -> GetSFunc): the target's OWN
/// script chain only — engine-global script functions do NOT resolve
/// (unlike GetFuncRecursive). Creation callbacks (PSF_Construction /
/// PSF_Initialize) use this; resolving a same-name scenario global here
/// would recurse (a def without Initialize must be a silent miss).
pub(crate) fn call_world_object_own_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, false, None)
}

fn call_world_object_function_with(
    target: ObjectId,
    function: &str,
    args: &[Value],
    host_fallback: bool,
    include_globals: bool,
    script_override: Option<Arc<ScriptEngine>>,
) -> Option<Result<Value, RuntimeError>> {
    let prep = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut().as_mut().and_then(|context| {
            context.prepare_nested_call(
                target,
                function,
                host_fallback,
                include_globals,
                script_override,
            )
        })
    })?;
    let NestedCallPrep {
        script,
        local_vars,
        origin,
    } = prep;
    let entry_locals = local_vars.clone();
    // LIVE local cells shared across every session on this object within
    // the outer call (C++ mutates the one live C4Object): the first
    // session seeds them from the snapshot, deeper sessions reuse them —
    // and their writes are visible mid-call. The creating session owns
    // cleanup.
    let (cells, created_session) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match borrow.as_mut() {
            Some(context) => match context.session_local_cells.get(&target) {
                Some(cells) => (cells.clone(), false),
                None => {
                    let cells = lc_script::LocalCells::from_local_vars(&local_vars);
                    context
                        .session_local_cells
                        .insert(target, cells.clone());
                    (cells, true)
                }
            },
            None => (lc_script::LocalCells::from_local_vars(&local_vars), false),
        }
    });
    // The HOST_CONTEXT borrow is released here: the nested VM's host
    // functions re-borrow it against the swapped-in scope.
    let call = script.call_with_cells_and_this(
        function,
        args,
        &cells,
        object_reference_value(target),
    );
    if created_session {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.session_local_cells.remove(&target);
            }
        });
    }
    let (result, stored_locals) = match call {
        Ok(value) => (Ok(value), cells.snapshot()),
        // Partial side effects before the error still fold (C++ mutates
        // live state) — the shared cells carry every write made before
        // the unwind.
        Err(lc_script::ScriptError::Runtime(err)) => (Err(err), cells.snapshot()),
        Err(other) => (
            Err(RuntimeError::new(other.to_string())),
            cells.snapshot(),
        ),
    };
    if let Some(origin) = origin {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                // Writes made by DEEPER same-scope calls (e.g. a
                // synchronous Fx*Start fired from inside this call) sit in
                // the foreign cells; they win over entries THIS call left
                // untouched — C++ mutates the one live object, so the
                // deepest write is simply the latest.
                let mut stored_locals = stored_locals;
                for ((object, name), slot) in &context.foreign_local_cells {
                    if *object != target {
                        continue;
                    }
                    // Unset locals read as nil (C4Value default) — a local
                    // the outer call never touched may be absent from its
                    // entry snapshot but nil-present in the VM result.
                    let outer_unchanged = entry_locals.get(name).unwrap_or(&Value::Nil)
                        == stored_locals.get(name).unwrap_or(&Value::Nil);
                    if outer_unchanged {
                        stored_locals.insert(name.clone(), slot.borrow().clone());
                    }
                }
                context.finish_nested_call(target, origin, stored_locals);
            }
        });
    } else {
        // Same-scope call (the target IS the in-flight active scope): the
        // outer VM owns the live locals, so fold this call's writes
        // through the foreign-cell channel — later nested calls overlay
        // them and the outcome fold persists them (C++ mutates the live
        // object directly).
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                for (name, value) in &stored_locals {
                    let slot = context.foreign_local_cell(target, name);
                    *slot.borrow_mut() = value.clone();
                }
            }
        });
    }
    Some(result)
}

/// `C4Value::operator bool` (C4Value.h:76,183-185): raw-data truthiness —
/// false only for nil, 0 and false; non-empty-ness is NOT required for
/// strings/arrays/maps, and no type conversion happens (unlike `getBool`).
pub(crate) fn value_raw_truthy(value: &Value) -> bool {
    !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false))
}

struct EffectHostContext {
    object: Option<ObjectScopeContext>,
    global: Option<EffectScopeContext>,
    /// LIVE local cells per object with an in-flight VM session: deeper
    /// nested calls onto the same object share them, so mid-call local
    /// writes are visible immediately (C++ mutates the live C4Object —
    /// the Talker's DoStartMovie sets sMovName and the synchronous
    /// FxMovieStart reads it within the same outer call).
    session_local_cells: HashMap<ObjectId, lc_script::LocalCells>,
    world: HostWorldContext,
    player_overrides: HashMap<i32, PlayerState>,
    player_commands: Vec<PlayerCommand>,
    team_home_base_rule: bool,
    pending_spawns: Vec<SpawnConfig>,
    pending_objects: HashMap<ObjectId, HostWorldObject>,
    pending_order: Vec<ObjectId>,
    pending_particles: Vec<ParticleCommand>,
    transfer_zone_commands: Vec<TransferZoneCommand>,
    pending_messages: Vec<MessageCommand>,
    pending_landscape_ops: Vec<LandscapeOperation>,
    audio: AudioRegistry,
    next_object_id: u64,
    trigger_game_over: bool,
    script_go_request: Option<bool>,
    game_over_triggered: bool,
    /// Saved `object` scopes of in-flight nested calls, one per nesting
    /// level (`None` = the level had no object scope). The active scope is
    /// always `object`; scopes move between locations by identity, so one
    /// object never has two scopes (no double-apply on fold).
    dormant_scopes: Vec<Option<ObjectScopeContext>>,
    /// Completed nested-call scopes by target, resumed on repeat calls and
    /// folded into `EffectContextOutcome::other_objects` in first-call order.
    nested_objects: HashMap<ObjectId, NestedScopeState>,
    nested_order: Vec<ObjectId>,
    /// Live cells handed to the VM for cross-object LocalN references
    /// (FnLocalN by-reference access, C4Script.cpp:4591-4605): seeded from
    /// the target's current locals, overlaid into nested calls, synced
    /// back after them, and folded into the outcomes. Targets whose scope
    /// is the suspended OUTER call see the pre-call snapshot (the same
    /// divergence prepare_nested_call documents).
    foreign_local_cells: HashMap<(ObjectId, String), lc_script::ValueCell>,
}

impl EffectHostContext {
    fn new(
        object: Option<HostObjectContext<'_>>,
        global_effects: Vec<EffectState>,
        world: HostWorldContext,
        next_object_id: u64,
        audio: AudioRegistry,
        game_over_triggered: bool,
    ) -> Self {
        let team_home_base_rule = world.team_home_base_rule();
        let object = object.map(|ctx| {
            let HostObjectContext {
                id,
                definition_id,
                container,
                status,
                energy,
                damage,
                construction,
                alive,
                in_liquid,
                own_mass,
                owner,
                controller,
                position,
                velocity,
                rotation,
                effects,
                action_name,
                action_ticks,
                action_data,
                action_phase,
                action_library,
                direction,
                command_direction,
                command_count,
                action_target,
                action_target2,
                vertices,
                graphics_overlays,
                base_graphics,
                category,
                ocf: _,
                ocf_base,
                crew_member,
                draw_transform,
                info_physical,
                temporary_physical,
                physical_changes,
                definition_physical,
            } = ctx;
            {
                let mut scope = ObjectScopeContext::new(
                    id,
                    container,
                    status,
                    energy,
                    damage,
                    construction,
                    alive,
                    in_liquid,
                    own_mass,
                    owner,
                    controller,
                    category,
                    position,
                    velocity,
                    rotation,
                    effects.to_vec(),
                    action_library,
                    action_name,
                    action_ticks,
                    action_data,
                    action_phase,
                    direction,
                    command_direction,
                    command_count,
                    action_target,
                    action_target2,
                    vertices.to_vec(),
                    ocf_base,
                    crew_member,
                    graphics_overlays,
                    base_graphics,
                    draw_transform,
                    info_physical,
                    temporary_physical,
                    physical_changes,
                    definition_physical,
                );
                scope.definition_id = definition_id;
                scope
            }
        });
        let global = Some(EffectScopeContext::new(global_effects));
        Self {
            object,
            global,
            world,
            player_overrides: HashMap::new(),
            player_commands: Vec::new(),
            team_home_base_rule,
            pending_spawns: Vec::new(),
            pending_objects: HashMap::new(),
            pending_order: Vec::new(),
            pending_particles: Vec::new(),
            transfer_zone_commands: Vec::new(),
            pending_messages: Vec::new(),
            pending_landscape_ops: Vec::new(),
            audio,
            next_object_id,
            trigger_game_over: false,
            script_go_request: None,
            game_over_triggered,
            dormant_scopes: Vec::new(),
            nested_objects: HashMap::new(),
            session_local_cells: HashMap::new(),
            nested_order: Vec::new(),
            foreign_local_cells: HashMap::new(),
        }
    }

    fn scope_mut(&mut self, scope: EffectScope) -> Result<&mut EffectScopeContext, RuntimeError> {
        match scope {
            EffectScope::Object => {
                self.object
                    .as_mut()
                    .map(|ctx| &mut ctx.effects)
                    .ok_or_else(|| {
                        RuntimeError::new(
                            "object effect operations require an active engine context",
                        )
                    })
            }
            EffectScope::Global => self.global.as_mut().ok_or_else(|| {
                RuntimeError::new("global effect operations require an active engine context")
            }),
        }
    }

    fn allocate_object_id(&mut self) -> ObjectId {
        let id = ObjectId::new(self.next_object_id);
        self.next_object_id += 1;
        id
    }

    fn register_spawn(&mut self, spawn: SpawnConfig, preview: HostWorldObject) {
        let id = preview.id;
        if !self.pending_objects.contains_key(&id) {
            self.pending_order.push(id);
        }
        self.pending_objects.insert(id, preview);
        self.pending_spawns.push(spawn);
    }

    fn register_particle(&mut self, command: ParticleCommand) {
        self.pending_particles.push(command);
    }

    /// `Some(known?)` when the engine attached its particle def registry,
    /// `None` for legacy fixture contexts. See `HostWorldContext`.
    fn particle_def_known(&self, name: &str) -> Option<bool> {
        self.world.particle_def_known(name)
    }

    fn register_transfer_zone_command(&mut self, command: TransferZoneCommand) {
        self.transfer_zone_commands.push(command);
    }

    fn register_message(&mut self, command: MessageCommand) {
        self.pending_messages.push(command);
    }

    fn register_landscape_operation(&mut self, operation: LandscapeOperation) {
        self.pending_landscape_ops.push(operation);
    }

    fn get_world_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        let mut object = if let Some(object) = self.pending_objects.get(&id) {
            object.clone()
        } else {
            self.world.get(id).cloned()?
        };
        // C++ mutates live state mid-call: reflect the freshest known
        // containment (Enter/Exit in the active or a finished nested
        // scope) so container-filtered searches see it (FnFindObject
        // vContainer, C4Script.cpp:2122-2127).
        if let Some(container) = self
            .object
            .as_ref()
            .filter(|scope| scope.id == id)
            .map(|scope| scope.current_container)
            .or_else(|| {
                self.nested_objects
                    .get(&id)
                    .map(|state| state.scope.current_container)
            })
        {
            object.container = container;
        }
        Some(object)
    }

    /// Drops a spawn queued in THIS call before it materializes. The id
    /// stays allocated (C++ objects removed in the creating call still
    /// consumed their Number).
    fn cancel_pending_spawn(&mut self, target: ObjectId) -> bool {
        let before = self.pending_spawns.len();
        self.pending_spawns.retain(|spawn| spawn.id != Some(target));
        let removed = self.pending_spawns.len() != before;
        if removed {
            self.pending_order.retain(|id| *id != target);
            self.pending_objects.remove(&target);
        }
        removed
    }

    /// The live cell for a FOREIGN object's named local (cross-object
    /// LocalN). Seeded from the freshest known value: an accumulated
    /// nested-call state first, the world snapshot otherwise.
    fn foreign_local_cell(&mut self, target: ObjectId, name: &str) -> lc_script::ValueCell {
        if let Some(cell) = self.foreign_local_cells.get(&(target, name.to_string())) {
            return cell.clone();
        }
        let seed = self
            .nested_objects
            .get(&target)
            .and_then(|state| state.local_vars.get(name).cloned())
            .or_else(|| {
                self.get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
                    .and_then(|locals| locals.get(name).cloned())
            })
            .unwrap_or(Value::Nil);
        let cell = lc_script::value_cell(seed);
        self.foreign_local_cells
            .insert((target, name.to_string()), cell.clone());
        cell
    }

    /// Cross-object LocalN writes must be visible to a later nested call
    /// on the same target (C++ mutates live state mid-call).
    fn overlay_foreign_cells(&self, target: ObjectId, locals: &mut HashMap<String, Value>) {
        for ((object, name), cell) in &self.foreign_local_cells {
            if *object == target {
                locals.insert(name.clone(), cell.borrow().clone());
            }
        }
    }

    /// ...and a nested call's writes must be visible to later LocalN reads.
    fn sync_foreign_cells(&mut self, target: ObjectId, locals: &HashMap<String, Value>) {
        for ((object, name), cell) in &self.foreign_local_cells {
            if *object == target {
                if let Some(value) = locals.get(name) {
                    *cell.borrow_mut() = value.clone();
                }
            }
        }
    }

    /// Phase 1 of a nested call (borrow held): resolve the target's script
    /// and move its scope to active. Function resolution follows
    /// `FindSameNameFunc` (C4Aul.cpp:130-148): the target def's own script
    /// function wins, engine (host) functions are the fallback, anything
    /// else is a silent miss (`None`).
    fn prepare_nested_call(
        &mut self,
        target: ObjectId,
        function: &str,
        host_fallback: bool,
        include_globals: bool,
        script_override: Option<Arc<ScriptEngine>>,
    ) -> Option<NestedCallPrep> {
        let world_object = self.get_world_object(target)?;
        // Namespaced calls (`obj->ID::Func`) run the NAMED def's script in
        // the target's scope (AB_CALLNS); plain calls resolve on the
        // target's own def.
        let script = match script_override {
            Some(script) => script,
            None => self
                .world
                .definition_script(world_object.definition_id())?
                .clone(),
        };
        // GetFuncRecursive walks the def script up to Game.ScriptEngine
        // (C4Aul.cpp): GLOBAL script functions resolve for nested calls
        // and effect callbacks alike (GoldRush's FxStayThereStart lives
        // in the scenario script, C4Effect.cpp:31-40).
        let resolvable = script.has_function(function)
            || (include_globals && script.has_global_function(function))
            || (host_fallback && script.has_host_function(function));
        if !resolvable {
            return None;
        }
        // VM sessions own their locals, so a call onto an in-flight scope
        // reads the pre-call snapshot (divergence noted in PORT_STATUS).
        let mut snapshot_locals = world_object
            .full_state()
            .map(|state| state.local_vars.clone())
            .unwrap_or_default();
        // Earlier cross-object LocalN writes are part of the target's
        // current state.
        self.overlay_foreign_cells(target, &mut snapshot_locals);
        if self.object.as_ref().map(ObjectScopeContext::id) == Some(target) {
            return Some(NestedCallPrep {
                script,
                local_vars: snapshot_locals,
                origin: None,
            });
        }
        if let Some(index) = self
            .dormant_scopes
            .iter()
            .position(|slot| slot.as_ref().map(ObjectScopeContext::id) == Some(target))
        {
            let scope = self.dormant_scopes[index].take();
            self.dormant_scopes.push(self.object.take());
            self.object = scope;
            return Some(NestedCallPrep {
                script,
                local_vars: snapshot_locals,
                origin: Some(NestedScopeOrigin::Dormant(index)),
            });
        }
        let (scope, mut local_vars) = match self.nested_objects.remove(&target) {
            Some(state) => (state.scope, state.local_vars),
            None => self.nested_scope_for(&world_object)?,
        };
        self.overlay_foreign_cells(target, &mut local_vars);
        self.dormant_scopes.push(self.object.take());
        self.object = Some(scope);
        Some(NestedCallPrep {
            script,
            local_vars,
            origin: Some(NestedScopeOrigin::Completed),
        })
    }

    /// A fresh nested scope from the world snapshot. `None` for objects
    /// without a full-state snapshot (pending spawns of the same call).
    fn nested_scope_for(
        &self,
        object: &HostWorldObject,
    ) -> Option<(ObjectScopeContext, HashMap<String, Value>)> {
        let metadata = self.world.definition_metadata(object.definition_id())?;
        let state = object.full_state()?;
        let mut scope = ObjectScopeContext::new(
            object.id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.alive,
            state.in_liquid,
            state.own_mass,
            state.owner,
            state.controller,
            state.category,
            state.position,
            state.velocity,
            state.rotation,
            state.effects.clone(),
            metadata.action_library.clone(),
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            state.vertices.clone(),
            metadata.ocf_base,
            metadata.crew_member,
            state.graphics_overlays.clone(),
            state.base_graphics.clone(),
            state.draw_transform,
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            metadata.physical,
        );
        scope.definition_id = Some(object.definition_id().to_string());
        Some((scope, state.local_vars.clone()))
    }

    /// Phase 3 of a nested call (borrow re-taken): move the finished scope
    /// back to where it came from. Completed scopes keep `local_vars` for
    /// resumption and the outcome fold.
    fn finish_nested_call(
        &mut self,
        target: ObjectId,
        origin: NestedScopeOrigin,
        local_vars: HashMap<String, Value>,
    ) {
        // The call's writes become visible to later cross-object LocalN
        // reads on the same target.
        self.sync_foreign_cells(target, &local_vars);
        let finished = self.object.take();
        self.object = self.dormant_scopes.pop().unwrap_or(None);
        match origin {
            NestedScopeOrigin::Dormant(index) => {
                if let Some(slot) = self.dormant_scopes.get_mut(index) {
                    *slot = finished;
                }
            }
            NestedScopeOrigin::Completed => {
                if let Some(scope) = finished {
                    if !self.nested_order.contains(&target) {
                        self.nested_order.push(target);
                    }
                    self.nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
            }
        }
    }

    /// The scope currently holding `target`'s pending writes: the active
    /// scope, a dormant (in-flight outer) scope, or a completed nested one.
    fn object_scope(&self, target: ObjectId) -> Option<&ObjectScopeContext> {
        self.object
            .as_ref()
            .filter(|scope| scope.id == target)
            .or_else(|| {
                self.dormant_scopes
                    .iter()
                    .flatten()
                    .find(|scope| scope.id == target)
            })
            .or_else(|| self.nested_objects.get(&target).map(|state| &state.scope))
    }

    fn object_scope_mut(&mut self, target: ObjectId) -> Option<&mut ObjectScopeContext> {
        if self.object.as_ref().map(ObjectScopeContext::id) == Some(target) {
            return self.object.as_mut();
        }
        if self
            .dormant_scopes
            .iter()
            .flatten()
            .any(|scope| scope.id == target)
        {
            return self
                .dormant_scopes
                .iter_mut()
                .flatten()
                .find(|scope| scope.id == target);
        }
        self.nested_objects
            .get_mut(&target)
            .map(|state| &mut state.scope)
    }

    /// Materializes a nested scope for `target` so per-object writes (menus)
    /// can fold through the standard nested-outcome pipeline even when no
    /// script call ever ran on the target this session. False for unknown
    /// objects and same-call pending spawns (no full-state snapshot yet).
    fn ensure_object_scope(&mut self, target: ObjectId) -> bool {
        if self.object_scope(target).is_some() {
            return true;
        }
        let Some(world_object) = self.get_world_object(target) else {
            return false;
        };
        let Some((scope, local_vars)) = self.nested_scope_for(&world_object) else {
            return false;
        };
        if !self.nested_order.contains(&target) {
            self.nested_order.push(target);
        }
        self.nested_objects
            .insert(target, NestedScopeState { scope, local_vars });
        true
    }

    /// The freshest menu state known for `target` mid-call: a pending write
    /// in any scope wins over the world snapshot (C++ mutates the live
    /// C4Object::Menu).
    fn object_menu(&self, target: ObjectId) -> Option<crate::ObjectMenuState> {
        match self
            .object_scope(target)
            .and_then(|scope| scope.pending_update.menu.clone())
        {
            Some(menu) => menu,
            None => self
                .get_world_object(target)
                .and_then(|object| object.full_state().and_then(|state| state.menu.clone())),
        }
    }

    /// Records a menu write for `target` (Some = open/replace, None =
    /// closed). False when no scope can be materialized for the target.
    fn set_object_menu(&mut self, target: ObjectId, menu: Option<crate::ObjectMenuState>) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.menu = Some(menu))
            .is_some()
    }

    /// C4Object::Status of `target` as the current call sees it.
    fn object_status_active(&self, target: ObjectId) -> bool {
        self.object_scope(target)
            .map(|scope| scope.status().is_active())
            .unwrap_or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.status().is_active())
                    .unwrap_or(false)
            })
    }

    /// Whether a nested call removed the object — the C++ Status re-check
    /// after `Check` (C4FindObject.cpp:186-199) against the deferred-destroy
    /// model.
    fn nested_object_destroyed(&self, id: ObjectId) -> bool {
        self.nested_objects
            .get(&id)
            .map(|state| state.scope.destroy || !state.scope.status.is_active())
            .unwrap_or(false)
    }

    fn world_object_ids(&self) -> Vec<ObjectId> {
        let mut ids = self.world.object_ids().to_vec();
        ids.extend(self.pending_order.iter().copied());
        ids
    }

    /// `cthr->Obj` for the executing host call: the FindObject family
    /// excludes the caller and searches caller-relative coordinates on
    /// local calls (C4Script.cpp:2115-2131).
    fn caller_scope(&self) -> Option<(ObjectId, Vector2)> {
        self.object
            .as_ref()
            .map(|scope| (scope.id, scope.current_position))
    }

    fn definition_category(&self, id: &str) -> Option<i32> {
        self.world.definition_category(id)
    }

    fn definition_metadata(&self, id: &str) -> Option<&DefinitionMetadata> {
        self.world.definition_metadata(id)
    }

    fn landscape_ref(&self) -> Option<&Landscape> {
        self.world.landscape_ref()
    }

    fn snapshot(&self, scope: EffectScope) -> Option<Vec<EffectState>> {
        match scope {
            EffectScope::Object => self.object.as_ref().map(|ctx| ctx.effects.snapshot()),
            EffectScope::Global => self.global.as_ref().map(EffectScopeContext::snapshot),
        }
    }

    fn player_ids(&self) -> &[i32] {
        self.world.player_ids()
    }

    fn player_state(&self, id: i32) -> Option<&PlayerState> {
        self.player_overrides
            .get(&id)
            .or_else(|| self.world.player(id))
    }

    fn player_state_mut(&mut self, id: i32) -> Option<&mut PlayerState> {
        if !self.player_overrides.contains_key(&id) {
            let state = self.world.player(id)?.clone();
            self.player_overrides.insert(id, state);
        }
        self.player_overrides.get_mut(&id)
    }

    fn record_player_command(&mut self, command: PlayerCommand) {
        self.player_commands.push(command);
    }

    fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    fn object_context_mut(&mut self) -> Option<&mut ObjectScopeContext> {
        self.object.as_mut()
    }

    fn object_context(&self) -> Option<&ObjectScopeContext> {
        self.object.as_ref()
    }

    #[allow(dead_code)]
    fn audio_mut(&mut self) -> &mut AudioRegistry {
        &mut self.audio
    }

    #[allow(dead_code)]
    fn audio(&self) -> &AudioRegistry {
        &self.audio
    }

    fn request_game_over(&mut self) -> bool {
        if self.game_over_triggered {
            return false;
        }
        self.game_over_triggered = true;
        self.trigger_game_over = true;
        true
    }

    fn into_commands(mut self) -> EffectContextOutcome {
        debug_assert!(
            self.dormant_scopes.is_empty(),
            "all nested calls must have finished before the context closes"
        );
        // Cross-object LocalN cells fold like any other foreign mutation:
        // merged into the target's outcome locals (cells hold the LATEST
        // value, after any nested calls), with cell-only targets getting a
        // locals-only outcome seeded from their current state.
        let mut cell_locals: HashMap<ObjectId, HashMap<String, Value>> = HashMap::new();
        for ((object, name), cell) in &self.foreign_local_cells {
            cell_locals
                .entry(*object)
                .or_default()
                .insert(name.clone(), cell.borrow().clone());
        }
        let mut other_objects = Vec::new();
        for id in mem::take(&mut self.nested_order) {
            let Some(NestedScopeState {
                scope,
                mut local_vars,
            }) = self.nested_objects.remove(&id)
            else {
                continue;
            };
            if let Some(cells) = cell_locals.remove(&id) {
                local_vars.extend(cells);
            }
            let mut update = scope.pending_update;
            // Mirror the outer call's unconditional local-vars store
            // (Definition::call_object_function).
            update.local_vars = Some(local_vars);
            other_objects.push(NestedObjectOutcome {
                object_id: id,
                effects: scope.effects.into_commands(),
                update: Some(update),
                commands: scope.queued_commands,
                command_operations: scope.command_operations,
                destroy: scope.destroy,
            });
        }
        // Cell-only targets (LocalN writes without any nested call): a
        // locals-only outcome, full map seeded from the current state so
        // the unconditional store does not drop untouched locals. Sorted
        // for a deterministic fold order.
        let mut cell_only: Vec<ObjectId> = cell_locals.keys().copied().collect();
        cell_only.sort_unstable();
        for id in cell_only {
            let Some(cells) = cell_locals.remove(&id) else {
                continue;
            };
            let mut local_vars = self
                .get_world_object(id)
                .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
                .unwrap_or_default();
            local_vars.extend(cells);
            let update = ObjectUpdate {
                local_vars: Some(local_vars),
                ..ObjectUpdate::default()
            };
            other_objects.push(NestedObjectOutcome {
                object_id: id,
                effects: Vec::new(),
                update: Some(update),
                commands: Vec::new(),
                command_operations: Vec::new(),
                destroy: false,
            });
        }
        let (object_effects, object_update, object_commands, command_operations, destroy) =
            match self.object {
                Some(object) => {
                    let update = if object.pending_update.is_empty() {
                        None
                    } else {
                        Some(object.pending_update)
                    };
                    (
                        object.effects.into_commands(),
                        update,
                        object.queued_commands,
                        object.command_operations,
                        object.destroy,
                    )
                }
                None => (Vec::new(), None, Vec::new(), Vec::new(), false),
            };

        let global = self
            .global
            .map(EffectScopeContext::into_commands)
            .unwrap_or_default();

        // Spawns of this call whose nested scope was destroyed before
        // materializing (create -> RemoveObject within one call, the
        // GoldRush TRPR temp) never reach the world; their ids stay
        // consumed like C++.
        let destroyed: std::collections::HashSet<ObjectId> = other_objects
            .iter()
            .filter(|outcome| outcome.destroy)
            .map(|outcome| outcome.object_id)
            .collect();
        if !destroyed.is_empty() {
            self.pending_spawns
                .retain(|spawn| spawn.id.is_none_or(|id| !destroyed.contains(&id)));
        }

        let audio_events = self.audio.take_events();
        let mut outcome = EffectContextOutcome::new(
            object_effects,
            global,
            object_update,
            object_commands,
            command_operations,
            destroy,
            None,
            None,
            self.pending_spawns,
            self.pending_landscape_ops,
            self.transfer_zone_commands,
            self.pending_messages,
            self.player_commands,
            AudioOutcome {
                state: self.audio,
                events: audio_events,
            },
            self.trigger_game_over,
            self.script_go_request,
            self.next_object_id,
        );
        outcome.particles = self.pending_particles;
        outcome.other_objects = other_objects;
        outcome
    }
}

struct EffectScopeContext {
    effects: Vec<EffectState>,
    commands: Vec<EffectCommand>,
}

impl EffectScopeContext {
    fn new(effects: Vec<EffectState>) -> Self {
        Self {
            effects,
            commands: Vec::new(),
        }
    }

    fn snapshot(&self) -> Vec<EffectState> {
        self.effects.clone()
    }

    // iIntervall/iTime stored verbatim (C4Effect.cpp:66-67).
    fn add_effect(&mut self, mut effect: EffectState) -> i32 {
        if effect.interval < 0 {
            effect.interval = 0;
        }
        if effect.timer < 0 {
            effect.timer = 0;
        }

        // C4Effect::New: same-name effects coexist; the number is the
        // per-object max + 1 (C4Effect.cpp:76-78) and is the script-side
        // handle AddEffect returns.
        effect.number = self
            .effects
            .iter()
            .map(|existing| existing.number)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);

        let mut insert_pos = 0;
        while insert_pos < self.effects.len()
            && self.effects[insert_pos].priority.abs() < effect.priority.abs()
        {
            insert_pos += 1;
        }

        self.effects.insert(insert_pos, effect.clone());
        self.commands.push(EffectCommand::add(effect.clone()));
        effect.number
    }

    fn effect_var(
        &mut self,
        effect_number: usize,
        var_index: usize,
        new_value: Option<EffectVarValue>,
    ) -> Option<EffectVarValue> {
        if effect_number == 0 {
            return None;
        }
        let index = self
            .effects
            .iter()
            .position(|effect| effect.number == effect_number as i32)?;
        let effect = &mut self.effects[index];
        if let Some(value) = new_value {
            effect.set_var(var_index, value);
            let updated = effect.clone();
            self.commands.push(EffectCommand::update(updated));
        }
        Some(effect.var(var_index))
    }

    fn remove_effect(
        &mut self,
        name_filter: Option<&str>,
        index: usize,
        no_callbacks: bool,
    ) -> bool {
        let position = if let Some(name) = name_filter {
            let mut remaining = index;
            self.effects.iter().position(|effect| {
                // FnRemoveEffect resolves named removals through the
                // wildcard-aware C4Effect::Get (C4Script.cpp:5494).
                if s_wildcard_match_ex(&effect.name, name) {
                    if remaining == 0 {
                        true
                    } else {
                        remaining -= 1;
                        false
                    }
                } else {
                    false
                }
            })
        } else {
            // No name: iIndex is the effect NUMBER (FnRemoveEffect,
            // C4Script.cpp:5502-5507 -> C4Effect::Get(iNumber, false),
            // C4Effect.cpp:240-256). Numbers start at 1, so 0 matches
            // nothing.
            let number = i32::try_from(index).unwrap_or(i32::MAX);
            (number > 0)
                .then(|| {
                    self.effects
                        .iter()
                        .position(|effect| effect.number == number)
                })
                .flatten()
        };

        let position = match position {
            Some(pos) => pos,
            None => return false,
        };

        let effect = self.effects.remove(position);
        let command = if no_callbacks {
            EffectCommand::remove_without_callbacks(effect.name)
        } else {
            EffectCommand::remove(effect.name)
        };
        self.commands.push(command);
        true
    }

    fn into_commands(self) -> Vec<EffectCommand> {
        self.commands
    }
}

struct ObjectScopeContext {
    id: ObjectId,
    definition_id: Option<String>,
    current_container: Option<ObjectId>,
    status: ObjectStatus,
    effects: EffectScopeContext,
    pending_update: ObjectUpdate,
    queued_commands: Vec<QueuedCommand>,
    command_count: usize,
    command_operations: Vec<CommandOperation>,
    destroy: bool,
    action_library: ActionLibrary,
    current_action_name: String,
    current_action_blocks_other_actions: bool,
    current_action_target: Option<ObjectId>,
    current_action_target2: Option<ObjectId>,
    current_action_data: i32,
    current_action_ticks: u32,
    current_action_phase: i32,
    current_energy: i32,
    current_damage: i32,
    current_construction: i32,
    current_alive: bool,
    current_in_liquid: bool,
    current_own_mass: i32,
    current_owner: i32,
    /// C4Object::Controller (C4Object.h:127) — cause tracing.
    current_controller: i32,
    current_category: i32,
    ocf_base: u32,
    crew_member: bool,
    current_direction: Direction,
    current_command_direction: CommandDirection,
    current_position: Vector2,
    /// Sub-pixel velocity in 16.16 fixed-point. Precision-aware velocity
    /// surfaces (`SetXDir`/`GetXDir`) read and write this directly so that
    /// fractional `C4Fixed` velocity survives the script boundary. Seeded from
    /// the whole-pixel velocity at scope entry (full sub-pixel fidelity on
    /// entry awaits the snapshot work, task B).
    current_fixed_velocity: FixedVec2,
    current_rotation: i32,
    vertices: Vec<ObjectVertex>,
    graphics_overlays: Vec<ObjectGraphicsOverlay>,
    base_graphics: Option<ObjectBaseGraphics>,
    current_draw_transform: Option<DrawTransform>,
    info_physical: Option<PhysicalInfo>,
    temporary_physical: Option<PhysicalInfo>,
    physical_changes: Vec<(String, i32)>,
    definition_physical: PhysicalInfo,
}

impl ObjectScopeContext {
    fn new(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        construction: i32,
        alive: bool,
        in_liquid: bool,
        own_mass: i32,
        owner: i32,
        controller: i32,
        category: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        effects: Vec<EffectState>,
        action_library: ActionLibrary,
        action_name: String,
        action_ticks: u32,
        action_data: i32,
        action_phase: i32,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: Vec<ObjectVertex>,
        ocf_base: u32,
        crew_member: bool,
        graphics_overlays: Vec<ObjectGraphicsOverlay>,
        base_graphics: Option<ObjectBaseGraphics>,
        draw_transform: Option<DrawTransform>,
        info_physical: Option<PhysicalInfo>,
        temporary_physical: Option<PhysicalInfo>,
        physical_changes: Vec<(String, i32)>,
        definition_physical: PhysicalInfo,
    ) -> Self {
        let blocks_other_actions = action_library.blocks_other_actions(&action_name);
        let clamped_damage = damage.max(0);
        let clamped_construction = construction.clamp(0, FULL_CON);
        Self {
            definition_id: None,
            id,
            current_container: container,
            status,
            effects: EffectScopeContext::new(effects),
            pending_update: ObjectUpdate::default(),
            queued_commands: Vec::new(),
            command_count,
            command_operations: Vec::new(),
            destroy: false,
            action_library,
            current_action_name: action_name,
            current_action_blocks_other_actions: blocks_other_actions,
            current_action_target: action_target,
            current_action_target2: action_target2,
            current_action_data: action_data,
            current_action_ticks: action_ticks,
            current_action_phase: action_phase,
            current_energy: energy,
            current_damage: clamped_damage,
            current_construction: clamped_construction,
            current_alive: alive,
            current_in_liquid: in_liquid,
            current_own_mass: own_mass,
            current_owner: owner,
            current_controller: controller,
            current_category: category,
            ocf_base,
            crew_member,
            current_direction: direction,
            current_command_direction: command_direction,
            current_position: position,
            current_fixed_velocity: FixedVec2::from_ints(velocity.x, velocity.y),
            current_rotation: rotation.rem_euclid(360),
            vertices,
            graphics_overlays,
            base_graphics,
            current_draw_transform: draw_transform,
            info_physical,
            temporary_physical,
            physical_changes,
            definition_physical,
        }
    }

    fn id(&self) -> ObjectId {
        self.id
    }

    /// Record the full physical state into the pending update (applied
    /// wholesale by the engine — a cleared temp mode must overwrite).
    fn record_physicals(&mut self) {
        self.pending_update.physicals = Some(PhysicalsUpdate {
            info: self.info_physical,
            temporary: self.temporary_physical,
            changes: self.physical_changes.clone(),
        });
    }

    /// `C4Object::GetPhysical` (C4Object.cpp:2118-2134): temporary set when
    /// active (unless `permanent`), else info physicals, else definition.
    fn resolved_physical(&self, permanent: bool) -> PhysicalInfo {
        let temporary = (!permanent).then_some(self.temporary_physical).flatten();
        temporary
            .or(self.info_physical)
            .unwrap_or(self.definition_physical)
    }

    /// `FnGetPhysical` mode dispatch (C4Script.cpp:638-688, fair crew off).
    fn get_physical(&self, name: &str, mode: i32) -> Option<i32> {
        match mode {
            PHYS_CURRENT => self.resolved_physical(false).value_by_name(name),
            PHYS_PERMANENT => {
                // Info objects only (C4Script.cpp:668).
                if !self.crew_member {
                    return None;
                }
                self.info_physical
                    .unwrap_or(self.definition_physical)
                    .value_by_name(name)
            }
            PHYS_TEMPORARY => {
                // Info objects only, and only in temporary mode
                // (C4Script.cpp:680-682).
                if !self.crew_member {
                    return None;
                }
                self.temporary_physical
                    .and_then(|physical| physical.value_by_name(name))
            }
            _ => None,
        }
    }

    /// `FnSetPhysical` mode dispatch (C4Script.cpp:557-601, fair crew off).
    fn set_physical(&mut self, name: &str, value: i32, mode: i32) -> bool {
        // Unknown names fail (C4Script.cpp:562).
        if PhysicalInfo::default().value_mut_by_name(name).is_none() {
            return false;
        }
        match mode {
            PHYS_CURRENT => {
                // Temporary mode or info objects only (C4Script.cpp:569).
                if let Some(temporary) = self.temporary_physical.as_mut() {
                    temporary.set_by_name(name, value);
                } else if self.crew_member {
                    let definition_physical = self.definition_physical;
                    self.info_physical
                        .get_or_insert(definition_physical)
                        .set_by_name(name, value);
                } else {
                    return false;
                }
                self.record_physicals();
                true
            }
            PHYS_PERMANENT => {
                // Info objects only (C4Script.cpp:576).
                if !self.crew_member {
                    return false;
                }
                let definition_physical = self.definition_physical;
                self.info_physical
                    .get_or_insert(definition_physical)
                    .set_by_name(name, value);
                self.record_physicals();
                true
            }
            PHYS_TEMPORARY | PHYS_STACK_TEMPORARY => {
                // Auto-switch to temporary mode (C4Script.cpp:587-591).
                let base = self.resolved_physical(false);
                let temporary = self.temporary_physical.get_or_insert(base);
                // PHYS_StackTemporary remembers the old value
                // (C4Script.cpp:593-594; C4InfoCore.cpp:333-337).
                if mode == PHYS_STACK_TEMPORARY {
                    if let Some(previous) = temporary.value_by_name(name) {
                        self.physical_changes.push((name.to_string(), previous));
                    }
                }
                self.temporary_physical
                    .as_mut()
                    .map(|physical| physical.set_by_name(name, value));
                self.record_physicals();
                true
            }
            _ => false,
        }
    }

    /// `C4Object::TrainPhysical` (C4Object.cpp:2136-2146) over the scope
    /// copies; trains stacked previous values too (C4InfoCore.cpp:309-317).
    fn train_physical(&mut self, name: &str, train_by: i32, max_train: i32) -> bool {
        if PhysicalInfo::default().value_mut_by_name(name).is_none() {
            return false;
        }
        let mut trained = false;
        if let Some(temporary) = self.temporary_physical.as_mut() {
            if let Some(value) = temporary.value_mut_by_name(name) {
                PhysicalInfo::train_value(value, train_by, max_train);
            }
            for (_, previous) in self
                .physical_changes
                .iter_mut()
                .filter(|(changed, _)| changed.eq_ignore_ascii_case(name))
            {
                PhysicalInfo::train_value(previous, train_by, max_train);
            }
            trained = true;
        }
        if self.crew_member {
            let definition_physical = self.definition_physical;
            let info = self.info_physical.get_or_insert(definition_physical);
            if let Some(value) = info.value_mut_by_name(name) {
                PhysicalInfo::train_value(value, train_by, max_train);
            }
            trained = true;
        }
        if trained {
            self.record_physicals();
        }
        trained
    }

    /// `FnResetPhysical` (C4Script.cpp:613-636).
    fn reset_physical(&mut self, name: Option<&str>) -> bool {
        // Only in temporary mode (C4Script.cpp:619).
        if self.temporary_physical.is_none() {
            return false;
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            if PhysicalInfo::default().value_mut_by_name(name).is_none() {
                return false;
            }
            // Undo the last registered change for this physical
            // (C4InfoCore.cpp:339-351).
            let Some(position) = self
                .physical_changes
                .iter()
                .rposition(|(changed, _)| changed.eq_ignore_ascii_case(name))
            else {
                return false;
            };
            let (_, previous) = self.physical_changes.remove(position);
            self.temporary_physical
                .as_mut()
                .map(|physical| physical.set_by_name(name, previous));
            // Keep temporary mode while other changes remain or the set
            // still deviates from the reference (C4Script.cpp:628;
            // C4InfoCore.cpp:319-331).
            let reference = self.resolved_physical(true);
            let deviates = self
                .temporary_physical
                .map(|physical| physical != reference)
                .unwrap_or(false);
            if !self.physical_changes.is_empty() || deviates {
                self.record_physicals();
                return true;
            }
        }
        // Full reset (C4Script.cpp:631-635).
        self.temporary_physical = None;
        self.physical_changes.clear();
        self.record_physicals();
        true
    }

    fn status(&self) -> ObjectStatus {
        self.pending_update.status.unwrap_or(self.status)
    }

    fn set_status(&mut self, status: ObjectStatus) {
        self.status = status;
        self.pending_update.status = Some(status);
    }

    fn owner(&self) -> i32 {
        self.pending_update.owner.unwrap_or(self.current_owner)
    }

    fn set_owner(&mut self, owner: i32) {
        self.current_owner = owner;
        self.pending_update.owner = Some(owner);
        // C4Object::SetOwner "automatically updates controller"
        // (C4Object.cpp:5499-5500).
        self.set_controller(owner);
    }

    /// C4Object::Controller (FnGetController, C4Script.cpp:1316-1320).
    fn controller(&self) -> i32 {
        self.pending_update
            .controller
            .unwrap_or(self.current_controller)
    }

    /// FnSetController (C4Script.cpp:1322-1331).
    fn set_controller(&mut self, controller: i32) {
        self.current_controller = controller;
        self.pending_update.controller = Some(controller);
    }

    /// C4Player::MakeCrewMember adds the object to the crew
    /// (C4Player.cpp:1195-1196); the port keys crew off Owner +
    /// crew_member.
    fn set_crew_member(&mut self, crew_member: bool) {
        self.crew_member = crew_member;
        self.pending_update.crew_member = Some(crew_member);
    }

    /// SetPortrait's source-def bookkeeping (FnSetPortrait,
    /// C4Script.cpp:5333); the visual payload is unmodeled.
    fn set_portrait_source(&mut self, source: String) {
        self.pending_update.portrait_source = Some(source);
    }

    fn portrait_source_override(&self) -> Option<&String> {
        self.pending_update.portrait_source.as_ref()
    }

    /// FnSetSolidMask bookkeeping (C4Script.cpp:271-278).
    fn set_solid_mask_rect(&mut self, rect: crate::DefinitionTargetRect) {
        self.pending_update.solid_mask_override = Some(rect);
    }

    fn alive(&self) -> bool {
        self.pending_update.alive.unwrap_or(self.current_alive)
    }

    /// The cached InLiquid flag (scripts cannot set it; only
    /// FnSetPosition re-derives it, C4Script.cpp:475).
    fn in_liquid(&self) -> bool {
        self.current_in_liquid
    }

    fn own_mass(&self) -> i32 {
        self.current_own_mass
    }

    /// SetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass.
    fn set_own_mass(&mut self, own_mass: i32) {
        self.current_own_mass = own_mass;
        self.pending_update.own_mass = Some(own_mass);
    }

    fn set_alive(&mut self, alive: bool) {
        self.current_alive = alive;
        self.pending_update.alive = Some(alive);
    }

    fn category(&self) -> i32 {
        self.pending_update
            .category
            .unwrap_or(self.current_category)
    }

    fn set_category(&mut self, category: i32) {
        let normalized = crate::normalize_category(category, self.current_category);
        self.current_category = normalized;
        self.pending_update.category = Some(normalized);
    }

    fn clear_command_stack(&mut self) {
        self.command_operations.push(CommandOperation::Clear);
        self.command_count = 0;
    }

    fn push_command_front(&mut self, request: CommandRequest) -> bool {
        if self.command_count >= MAX_COMMAND_STACK {
            return false;
        }
        self.command_operations
            .push(CommandOperation::PushFront(request));
        self.command_count += 1;
        true
    }

    fn push_command_back(&mut self, request: CommandRequest) -> bool {
        if self.command_count >= MAX_COMMAND_STACK {
            return false;
        }
        self.command_operations
            .push(CommandOperation::PushBack(request));
        self.command_count += 1;
        true
    }

    fn ocf(&self) -> u32 {
        let alive = self.alive();
        let status = self.status();
        let is_contained = self.container().is_some();
        ocf::compute(
            self.ocf_base,
            self.crew_member,
            alive,
            status,
            is_contained,
            self.construction(),
        )
    }

    fn container(&self) -> Option<ObjectId> {
        match self.pending_update.container {
            Some(container) => container,
            None => self.current_container,
        }
    }

    fn set_container(&mut self, container: Option<ObjectId>) {
        if self.container() == container {
            return;
        }
        self.current_container = container;
        self.pending_update.container = Some(container);
    }

    fn mark_destroy(&mut self) {
        self.destroy = true;
    }

    fn update_effective_action(&mut self, action: &str) -> bool {
        let previous_name = self.current_action_name.clone();
        let previous_procedure = self.action_library.procedure_for_action(&previous_name);
        self.current_action_name = action.to_string();
        self.current_action_blocks_other_actions = self.action_library.blocks_other_actions(action);
        let next_procedure = self.action_library.procedure_for_action(action);
        previous_name != action && previous_procedure != next_procedure
    }

    fn effective_action_name(&self) -> &str {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return name;
            }
        }
        &self.current_action_name
    }

    fn effective_procedure_name(&self) -> Option<&str> {
        let action = self.effective_action_name();
        self.action_library.procedure_name_for_action(action)
    }

    fn effective_blocks_other_actions(&self) -> bool {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return self.action_library.blocks_other_actions(name);
            }
        }
        self.current_action_blocks_other_actions
    }

    fn effective_action_target(&self, index: usize) -> Option<ObjectId> {
        if let Some(update) = self.pending_update.action.as_ref() {
            match index {
                0 => {
                    if let Some(target) = update.target {
                        return target;
                    }
                }
                1 => {
                    if let Some(target) = update.target2 {
                        return target;
                    }
                }
                _ => return None,
            }
        }

        match index {
            0 => self.current_action_target,
            1 => self.current_action_target2,
            _ => None,
        }
    }

    fn effective_action_ticks(&self) -> u32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(ticks) = update.ticks {
                return ticks;
            }
        }
        self.current_action_ticks
    }

    fn effective_action_procedure(&self) -> ActionProcedure {
        let action = self.effective_action_name();
        self.action_library.procedure_for_action(action)
    }

    #[allow(dead_code)]
    fn effective_action_data(&self) -> i32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(data) = update.data {
                return data;
            }
        }
        self.current_action_data
    }

    fn set_action_data(&mut self, data: i32) {
        if self.current_action_data == data {
            if let Some(existing) = self
                .pending_update
                .action
                .as_ref()
                .and_then(|update| update.data)
            {
                if existing == data {
                    return;
                }
            } else {
                return;
            }
        }
        self.current_action_data = data;
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_data(data);
    }

    fn reset_action_data(&mut self) {
        self.set_action_data(0);
    }

    fn action_phase(&self) -> i32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(phase) = update.phase {
                return phase;
            }
        }
        self.current_action_phase
    }

    fn set_action_phase(&mut self, phase: i32) {
        if self.current_action_phase == phase {
            if let Some(existing) = self
                .pending_update
                .action
                .as_ref()
                .and_then(|update| update.phase)
            {
                if existing == phase {
                    return;
                }
            } else {
                return;
            }
        }
        self.current_action_phase = phase;
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_phase(phase);
    }

    fn set_action_ticks(&mut self, ticks: u32) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_ticks(ticks);
        self.current_action_ticks = ticks;
    }

    fn reset_action_ticks(&mut self) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_ticks(0);
        self.current_action_ticks = 0;
    }

    fn energy(&self) -> i32 {
        self.pending_update.energy.unwrap_or(self.current_energy)
    }

    fn set_energy(&mut self, energy: i32) {
        self.current_energy = energy;
        self.pending_update.energy = Some(energy);
    }

    fn damage(&self) -> i32 {
        self.pending_update.damage.unwrap_or(self.current_damage)
    }

    fn set_damage(&mut self, damage: i32) {
        let clamped = damage.max(0);
        self.current_damage = clamped;
        self.pending_update.damage = Some(clamped);
    }

    fn adjust_damage(&mut self, delta: i32) -> i32 {
        let current = self.damage();
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        self.set_damage(next);
        next
    }

    fn construction(&self) -> i32 {
        self.pending_update
            .construction
            .unwrap_or(self.current_construction)
    }

    fn set_construction(&mut self, construction: i32) {
        let clamped = construction.clamp(0, FULL_CON);
        self.current_construction = clamped;
        self.pending_update.construction = Some(clamped);
    }

    fn adjust_construction(&mut self, delta: i32) -> i32 {
        let current = self.construction();
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        } else if next > FULL_CON {
            next = FULL_CON;
        }
        self.set_construction(next);
        next
    }

    fn direction(&self) -> Direction {
        self.pending_update
            .direction
            .unwrap_or(self.current_direction)
    }

    fn set_direction(&mut self, direction: Direction) {
        if self.direction() == direction {
            return;
        }
        self.current_direction = direction;
        self.pending_update.direction = Some(direction);
    }

    fn rotation(&self) -> i32 {
        self.pending_update
            .rotation
            .unwrap_or(self.current_rotation)
    }

    fn set_rotation(&mut self, rotation: i32) {
        let normalized = rotation.rem_euclid(360);
        if self.rotation() == normalized {
            return;
        }
        self.current_rotation = normalized;
        self.pending_update.rotation = Some(normalized);
    }

    fn command_direction(&self) -> CommandDirection {
        self.pending_update
            .command_direction
            .unwrap_or(self.current_command_direction)
    }

    fn set_command_direction(&mut self, command_direction: CommandDirection) {
        if self.command_direction() == command_direction {
            return;
        }
        self.current_command_direction = command_direction;
        self.pending_update.command_direction = Some(command_direction);
    }

    fn fixed_velocity(&self) -> FixedVec2 {
        self.pending_update
            .fixed_velocity
            .unwrap_or(self.current_fixed_velocity)
    }

    /// Set the sub-pixel velocity and keep the whole-pixel mirror derived from
    /// it (`fixtoi`), so both `GetXDir`-style reads and the integer snapshot
    /// stay consistent with the `C4Fixed` source of truth.
    fn set_fixed_velocity(&mut self, velocity: FixedVec2) {
        self.current_fixed_velocity = velocity;
        self.pending_update.fixed_velocity = Some(velocity);
        // Keep the whole-pixel mirror consistent (fixtoi of the fixed value).
        self.pending_update.velocity = Some(Vector2::new(velocity.int_x(), velocity.int_y()));
    }

    /// Component-only dir write (FnSetXDir/FnSetYDir): stages just the
    /// touched component — the fold lands it on the object's TRUE fixed
    /// velocity without disturbing the other component.
    fn set_fixed_velocity_component(&mut self, component: VelocityComponent, value: C4Fixed) {
        let mut current = self.fixed_velocity();
        component.assign_fixed(&mut current, value);
        self.current_fixed_velocity = current;
        match component {
            VelocityComponent::X => self.pending_update.fixed_velocity_x = Some(value),
            VelocityComponent::Y => self.pending_update.fixed_velocity_y = Some(value),
        }
    }

    /// Angular velocity (`rdir`) as seen by `GetRDir`. The script object snapshot
    /// does not yet carry the live `rdir`, so the entry value reads as zero; a
    /// `SetRDir` earlier in the same call is reflected via the pending update.
    /// (Threading the live `rdir` into the script snapshot is a shared follow-up
    /// with full `GetXDir` entry fidelity.)
    fn rotation_velocity(&self) -> C4Fixed {
        self.pending_update
            .rotation_velocity
            .unwrap_or(C4Fixed::ZERO)
    }

    fn set_rotation_velocity(&mut self, rotation_velocity: C4Fixed) {
        self.pending_update.rotation_velocity = Some(rotation_velocity);
    }

    /// Queue a cyclic contents rotation so `new_front` becomes the first
    /// content (C4ObjectList::ShiftContents, C4ObjectList.cpp:815-833).
    fn shift_contents_front(&mut self, new_front: ObjectId) {
        self.pending_update.contents_front = Some(new_front);
    }

    fn effective_position(&self) -> Vector2 {
        self.pending_update
            .position
            .unwrap_or(self.current_position)
    }

    fn set_position(&mut self, position: Vector2) {
        if self.effective_position() == position && self.pending_update.position.is_none() {
            return;
        }
        self.current_position = position;
        self.pending_update.position = Some(position);
    }

    fn vertices(&self) -> &[ObjectVertex] {
        if let Some(vertices) = self.pending_update.vertices.as_ref() {
            vertices
        } else {
            &self.vertices
        }
    }

    fn set_graphics_overlay(&mut self, overlay: ObjectGraphicsOverlay) -> bool {
        let mut change = false;
        if let Some(existing) = self
            .graphics_overlays
            .iter_mut()
            .find(|existing| existing.id == overlay.id)
        {
            if *existing != overlay {
                *existing = overlay;
                change = true;
            }
        } else {
            self.graphics_overlays.push(overlay);
            self.graphics_overlays.sort_by_key(|overlay| overlay.id);
            change = true;
        }

        if change {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
        }
        change
    }

    fn remove_graphics_overlay(&mut self, id: i32) -> bool {
        let original_len = self.graphics_overlays.len();
        self.graphics_overlays.retain(|overlay| overlay.id != id);
        if self.graphics_overlays.len() != original_len {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
            true
        } else {
            false
        }
    }

    fn set_base_graphics(&mut self, base: Option<ObjectBaseGraphics>) -> bool {
        if self.base_graphics == base {
            return false;
        }
        self.base_graphics = base.clone();
        self.pending_update.base_graphics = Some(base);
        true
    }

    fn draw_transform(&self) -> Option<DrawTransform> {
        self.pending_update
            .draw_transform
            .unwrap_or(self.current_draw_transform)
    }

    fn set_draw_transform(&mut self, transform: Option<DrawTransform>) {
        if self.draw_transform() == transform {
            return;
        }
        self.current_draw_transform = transform;
        self.pending_update.draw_transform = Some(transform);
    }

    fn set_overlay_transform(&mut self, id: i32, transform: Option<DrawTransform>) -> bool {
        let mut changed = false;
        if let Some(existing) = self
            .graphics_overlays
            .iter_mut()
            .find(|overlay| overlay.id == id)
        {
            if existing.transform != transform {
                existing.transform = transform;
                changed = true;
            }
        } else {
            return false;
        }

        if changed {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
        }
        true
    }

    fn overlay_transform(&self, id: i32) -> Option<Option<DrawTransform>> {
        self.graphics_overlays
            .iter()
            .find(|overlay| overlay.id == id)
            .map(|overlay| overlay.transform)
    }

    fn set_action_target(&mut self, index: usize, target: Option<ObjectId>) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        match index {
            0 => {
                update.set_target(target);
                self.current_action_target = target;
            }
            1 => {
                update.set_target2(target);
                self.current_action_target2 = target;
            }
            _ => {}
        }
    }

    /// `C4Object::DoEnergy` (C4Object.cpp:1345-1364): percent scale unless
    /// fExact (`iChange *= C4MaxPhysical/100`), clamped to
    /// 0..GetPhysical()->Energy. Physical-less fixture defs (energy 0)
    /// keep the raw value unclamped — real content always carries a
    /// [Physical] Energy.
    fn adjust_energy(&mut self, delta: i32, exact: bool) -> i32 {
        let delta = if exact {
            delta
        } else {
            delta.saturating_mul(LEGACY_MAX_PHYSICAL / 100)
        };
        let mut next = self.energy().saturating_add(delta).max(0);
        let max_energy = self.resolved_physical(false).energy;
        if max_energy > 0 {
            next = next.min(max_energy);
        }
        self.set_energy(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandId, CommandOperation};
    use crate::ocf;
    use crate::ActionSpec;
    use crate::AudioCommand;
    use lc_resources::C4_MAX_PHYSICAL;
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::fmt;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{subscriber, Level};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    const EXPECTED_HOST_FUNCTIONS: &[&str] = &[
        "Abs",
        "ActIdle",
        "AddCommand",
        "AddEffect",
        "AddMenuItem",
        "AddMessage",
        "AnyContainer",
        "AppendCommand",
        "BlastFree",
        "BoundBy",
        "C4Id",
        "Call",
        "CastBackParticles",
        "CastParticles",
        "ChangeDef",
        "ClearParticles",
        "CloseMenu",
        "Contained",
        "Contents",
        "ContentsCount",
        "Cos",
        "CreateArray",
        "CreateConstruction",
        "CreateContents",
        "CreateMenu",
        "CreateObject",
        "CreateParticle",
        "CustomMessage",
        "DebugLog",
        "DefinitionCall",
        "DigFree",
        "DigFreeRect",
        "Distance",
        "DoCon",
        "DoDamage",
        "DoEnergy",
        "DoHomebaseMaterial",
        "DoHomebaseProduction",
        "EffectVar",
        "EnergyCheck",
        "Enter",
        "Exit",
        "ExtractMaterialAmount",
        "FindContents",
        "FindObject",
        "FindObject2",
        "FindObjectOwner",
        "FindObjects",
        "FindOtherContents",
        "FinishCommand",
        "Fling",
        "Format",
        "FrameCounter",
        "FreeRect",
        "GBackLiquid",
        "GBackSemiSolid",
        "GBackSky",
        "GBackSolid",
        "GameCall",
        "GameCallEx",
        "GameOver",
        "GetActMapVal",
        "GetActTime",
        "GetAction",
        "GetActionData",
        "GetActionTarget",
        "GetAlive",
        "GetBreath",
        "GetCategory",
        "GetChar",
        "GetClimate",
        "GetColorDw",
        "GetComDir",
        "GetCommand",
        "GetComponent",
        "GetCon",
        "GetContact",
        "GetController",
        "GetCrew",
        "GetCrewCount",
        "GetCrewEnabled",
        "GetCursor",
        "GetDefCoreVal",
        "GetDir",
        "GetEffect",
        "GetEffectCount",
        "GetEnergy",
        "GetGravity",
        "GetHiRank",
        "GetHomebaseMaterial",
        "GetHomebaseProduction",
        "GetID",
        "GetIndexOf",
        "GetKeys",
        "GetLength",
        "GetMass",
        "GetMaterial",
        "GetMaterialVal",
        "GetMenu",
        "GetName",
        "GetOCF",
        "GetObjectLayer",
        "GetObjectStatus",
        "GetObjectVal",
        "GetOwner",
        "GetPath",
        "GetPhase",
        "GetPhysical",
        "GetPlayerByIndex",
        "GetPlayerCount",
        "GetPlayerID",
        "GetPlayerName",
        "GetPlayerTeam",
        "GetPlayerType",
        "GetPlrDownDouble",
        "GetPlrKnowledge",
        "GetPlrValue",
        "GetPlrValueGain",
        "GetPortrait",
        "GetProcedure",
        "GetR",
        "GetRDir",
        "GetScore",
        "GetSelectCount",
        "GetTemperature",
        "GetType",
        "GetValues",
        "GetVertex",
        "GetVertexContact",
        "GetVertexNum",
        "GetViewCursor",
        "GetVisibility",
        "GetWealth",
        "GetWind",
        "GetX",
        "GetXDir",
        "GetY",
        "GetYDir",
        "GrabObjectInfo",
        "InLiquid",
        "IncinerateLandscape",
        "InsertMaterial",
        "Inside",
        "Jump",
        "LandscapeHeight",
        "LandscapeWidth",
        "Log",
        "MakeCrewMember",
        "Material",
        "Max",
        "Message",
        "Min",
        "Mod",
        "NoContainer",
        "ObjectCall",
        "ObjectCount",
        "ObjectCount2",
        "ObjectDistance",
        "ObjectSetAction",
        "OnFire",
        "PathFree",
        "PlayerMessage",
        "PlrMessage",
        "Pow",
        "PrivateCall",
        "ProtectedCall",
        "Punch",
        "PushParticles",
        "Random",
        "RemoveEffect",
        "RemoveObject",
        "ResetPhysical",
        "ScriptGo",
        "SelectMenuItem",
        "SetAction",
        "SetActionData",
        "SetActionTargets",
        "SetAlive",
        "SetBridgeActionData",
        "SetCategory",
        "SetClimate",
        "SetClrModulation",
        "SetColorDw",
        "SetComDir",
        "SetCommand",
        "SetComponent",
        "SetController",
        "SetCrewEnabled",
        "SetCursor",
        "SetDir",
        "SetEntrance",
        "SetGraphics",
        "SetGravity",
        "SetMass",
        "SetObjDrawTransform",
        "SetObjDrawTransform2",
        "SetObjectStatus",
        "SetOwner",
        "SetPhase",
        "SetPhysical",
        "SetPlrKnowledge",
        "SetPlrView",
        "SetPlrViewRange",
        "SetPortrait",
        "SetPosition",
        "SetR",
        "SetRDir",
        "SetShape",
        "SetSolidMask",
        "SetTemperature",
        "SetTransferZone",
        "SetVertex",
        "SetViewOffset",
        "SetVisibility",
        "SetWealth",
        "SetWind",
        "SetXDir",
        "SetYDir",
        "ShakeFree",
        "ShiftContents",
        "Sin",
        "Smoke",
        "Sound",
        "SoundLevel",
        "Sqrt",
        "Stuck",
        "TrainPhysical",
        "WildcardMatch",
    ];

    #[test]
    fn host_function_registration_matches_expected() {
        let mut engine = lc_script::Engine::new();
        register_host_functions(&mut engine);
        let actual = engine.host_function_names();
        let expected: Vec<String> = EXPECTED_HOST_FUNCTIONS
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(actual, expected);
    }

    fn empty_state() -> Value {
        let mut map = HashMap::new();
        map.insert("effects".into(), Value::Array(Vec::new()));
        Value::Proplist(map)
    }

    fn with_object_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_object_host_context_with_world(HostWorldContext::default(), func)
    }

    fn with_object_host_context_with_world<F, T>(
        world: HostWorldContext,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            world,
            1,
            func,
        )
    }

    fn with_environment_context<F, T>(
        settings: EnvironmentSettings,
        frame: u64,
        func: F,
    ) -> (Result<T, RuntimeError>, EnvironmentDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_environment_context(settings, frame);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    fn with_physics_context<F, T>(
        settings: PhysicsSettings,
        func: F,
    ) -> (Result<T, RuntimeError>, PhysicsDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_physics_context(settings);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    #[derive(Debug)]
    struct RecordedEvent {
        level: Level,
        target: String,
        message: String,
    }

    #[derive(Clone)]
    struct RecordingLayer {
        records: Arc<Mutex<Vec<RecordedEvent>>>,
    }

    impl RecordingLayer {
        fn new(records: Arc<Mutex<Vec<RecordedEvent>>>) -> Self {
            Self { records }
        }
    }

    impl<S> Layer<S> for RecordingLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            let message = visitor.message.unwrap_or_default();
            let record = RecordedEvent {
                level: *event.metadata().level(),
                target: event.metadata().target().to_string(),
                message,
            };
            self.records.lock().unwrap().push(record);
        }
    }

    #[derive(Default)]
    struct MessageVisitor {
        message: Option<String>,
    }

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                let mut text = format!("{value:?}");
                if let Some(stripped) = text
                    .strip_prefix('"')
                    .and_then(|inner| inner.strip_suffix('"'))
                {
                    text = stripped.to_string();
                }
                self.message = Some(text);
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.message = Some(value.to_string());
            }
        }
    }

    #[test]
    fn get_keys_returns_sorted_keys() {
        let mut map = HashMap::new();
        map.insert("beta".into(), Value::Int(2));
        map.insert("alpha".into(), Value::Int(1));
        let result = get_keys(&[Value::Proplist(map)]).expect("GetKeys succeeds");
        match result {
            Value::Array(entries) => {
                assert_eq!(
                    entries,
                    vec![Value::String("alpha".into()), Value::String("beta".into())]
                );
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    #[test]
    fn no_container_and_any_container_return_cpp_sentinels() {
        // FnNoContainer/FnAnyContainer return the FindObject container
        // sentinels NO_CONTAINER=124 / ANY_CONTAINER=123 (C4Object.h:83-84,
        // C4Script.cpp:6731-6732).
        assert_eq!(no_container(&[]).expect("NoContainer"), Value::Int(124));
        assert_eq!(any_container(&[]).expect("AnyContainer"), Value::Int(123));
    }

    #[test]
    fn act_idle_without_context_is_nil() {
        // FnActIdle returns nullopt -> nil without an object
        // (C4Script.cpp:1831-1836).
        assert_eq!(act_idle(&[]).expect("ActIdle"), Value::Nil);
    }

    #[test]
    fn falsy_zero_converts_to_any_parameter_type_like_cpp() {
        // Pre-strict3 callers reset any falsy parameter to nil before the
        // type check (C4AulExec.cpp:1372 `!pPars[i]` -> Set0), and nil
        // converts to every type (C4Value.cpp FnCnvGuess). TIPI's
        // Initialize calls SetGraphics(0) meaning "default graphics".
        match set_graphics(&[Value::Int(0)]) {
            Ok(_) => {}
            Err(error) => assert!(
                !error.message().contains("expected string"),
                "SetGraphics(0) must mean default graphics, got: {}",
                error.message()
            ),
        }
    }

    #[test]
    fn engine_function_parameters_default_to_nil_zero_like_cpp() {
        // Every C4Aul call carries 10 parameter slots, unfilled = nil
        // (C4Aul.h:104-121); nil converts to int 0, so GBackSolid() with no
        // arguments queries the object's own position (DRAI/WTFL/LAFL
        // action-start scripts call it bare).
        let solid = g_back_solid(&[]).expect("GBackSolid() succeeds");
        assert!(matches!(solid, Value::Bool(_)));
        let liquid = g_back_liquid(&[Value::Nil]).expect("GBackLiquid(nil) succeeds");
        assert!(matches!(liquid, Value::Bool(_)));
    }

    #[test]
    fn create_object_accepts_id_values_like_cpp() {
        // FnCreateObject's first parameter is a C4ID (C4Script.cpp:1892);
        // content passes id literals (BAS7/_ROA/NTIP Construction).
        let error = create_object(&[Value::C4Id("ROCK".into())])
            .expect_err("no engine context in unit test");
        assert!(
            !error.message().contains("expected string"),
            "id must be a valid definition argument, got: {}",
            error.message()
        );
    }

    #[test]
    fn get_keys_rejects_nil() {
        let error = get_keys(&[Value::Nil]).expect_err("GetKeys should fail for nil");
        assert_eq!(error.message(), "GetKeys(): map expected, got 0");
    }

    #[test]
    fn get_values_returns_entries_sorted_by_key() {
        let mut map = HashMap::new();
        map.insert("beta".into(), Value::Int(2));
        map.insert("alpha".into(), Value::Int(1));
        let result = get_values(&[Value::Proplist(map)]).expect("GetValues succeeds");
        match result {
            Value::Array(entries) => {
                assert_eq!(entries, vec![Value::Int(1), Value::Int(2)]);
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    #[test]
    fn get_values_rejects_nil() {
        let error = get_values(&[Value::Nil]).expect_err("GetValues should fail for nil");
        assert_eq!(error.message(), "GetValues(): map expected, got 0");
    }

    #[test]
    fn message_formats_and_registers_global_message() {
        let args = [
            Value::String("Score %03d".into()),
            Value::Nil,
            Value::Int(7),
        ];
        let (result, outcome) = with_object_host_context(|| message(&args));
        assert_eq!(result.expect("Message succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::Global);
                assert_eq!(spec.text, "Score 007");
                assert!(spec.target.is_none());
                assert!(spec.player.is_none());
            }
        }
    }

    #[test]
    fn message_with_speech_only_emits_audio() {
        let args = [Value::String("Hello$Horn".into())];
        let (result, outcome) = with_object_host_context(|| message(&args));
        assert_eq!(result.expect("Message succeeds"), Value::Bool(true));
        assert!(outcome.messages.is_empty());
        assert_eq!(outcome.audio.events.len(), 1);
        match &outcome.audio.events[0] {
            AudioCommand::PlaySound {
                name,
                volume,
                looped,
                ..
            } => {
                assert_eq!(name, "Horn");
                assert_eq!(*volume, 100);
                assert!(!looped);
            }
            other => panic!("expected PlaySound, got {other:?}"),
        }
    }

    #[test]
    fn player_message_targets_valid_player() {
        let mut player = PlayerState::default();
        player.id = 1;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(1), Value::String("Hi there".into())];
        let (result, outcome) =
            with_object_host_context_with_world(world, || player_message(&args));
        assert_eq!(result.expect("PlayerMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::GlobalPlayer);
                assert_eq!(spec.player, Some(1));
                assert_eq!(spec.text, "Hi there");
            }
        }
    }

    #[test]
    fn add_message_sets_multiple_flag() {
        let args = [Value::String("Queued".into())];
        let (result, outcome) = with_object_host_context(|| add_message(&args));
        assert_eq!(result.expect("AddMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.flags & FLAG_MULTIPLE, FLAG_MULTIPLE);
                assert_eq!(spec.text, "Queued");
            }
        }
    }

    #[test]
    fn plr_message_degrades_to_global_when_player_missing() {
        let args = [Value::String("Warning".into()), Value::Int(42)];
        let (result, outcome) = with_object_host_context(|| plr_message(&args));
        assert_eq!(result.expect("PlrMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::Global);
                assert!(spec.player.is_none());
                assert_eq!(spec.text, "Warning");
            }
        }
    }

    #[test]
    fn format_applies_legacy_placeholders() {
        let args = [
            Value::String("Crew %03d %i %s %v %%".into()),
            Value::Int(7),
            Value::String("CLNK".into()),
            Value::String("Ready".into()),
            Value::Int(5),
        ];
        let result = format_string(&args).expect("Format succeeds");
        assert_eq!(result, Value::String("Crew 007 CLNK Ready 5 %".into()));
    }

    #[test]
    fn get_type_reports_basic_value_kinds() {
        assert_eq!(
            get_type(&[Value::Nil]).expect("GetType succeeds"),
            Value::Int(C4V_ANY)
        );
        assert_eq!(
            get_type(&[Value::Int(7)]).expect("GetType succeeds"),
            Value::Int(C4V_INT)
        );
        assert_eq!(
            get_type(&[Value::Bool(true)]).expect("GetType succeeds"),
            Value::Int(C4V_BOOL)
        );
        assert_eq!(
            get_type(&[Value::String("Hi".into())]).expect("GetType succeeds"),
            Value::Int(C4V_STRING)
        );
        assert_eq!(
            get_type(&[Value::Array(vec![Value::Int(1)])]).expect("GetType succeeds"),
            Value::Int(C4V_ARRAY)
        );
        let mut map = HashMap::new();
        map.insert("key".into(), Value::Int(1));
        assert_eq!(
            get_type(&[Value::Proplist(map)]).expect("GetType succeeds"),
            Value::Int(C4V_MAP)
        );
    }

    #[test]
    fn create_array_allocates_nil_initialised_values() {
        let result = create_array(&[Value::Int(3)]).expect("CreateArray succeeds");
        assert_eq!(
            result,
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
        );
    }

    #[test]
    fn create_array_rejects_out_of_range_sizes() {
        let error = create_array(&[Value::Int(-1)]).expect_err("CreateArray rejects negative");
        assert!(error
            .message()
            .starts_with("CreateArray: invalid array size"));

        let error = create_array(&[Value::Int(LEGACY_MAX_ARRAY_SIZE + 1)])
            .expect_err("CreateArray rejects oversized");
        assert!(error
            .message()
            .starts_with("CreateArray: invalid array size"));
    }

    #[test]
    fn get_length_returns_lengths_for_supported_types() {
        let result = get_length(&[Value::String("abc".into())]).expect("GetLength succeeds");
        assert_eq!(result, Value::Int(3));

        let result =
            get_length(&[Value::Array(vec![Value::Int(1), Value::Int(2)])]).expect("array length");
        assert_eq!(result, Value::Int(2));

        let mut map = HashMap::new();
        map.insert("a".into(), Value::Int(1));
        map.insert("b".into(), Value::Bool(true));
        let result = get_length(&[Value::Proplist(map)]).expect("map length");
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn get_length_returns_nil_for_falsey_values() {
        assert_eq!(get_length(&[Value::Nil]).expect("nil handled"), Value::Nil);
        assert_eq!(
            get_length(&[Value::Bool(false)]).expect("false handled"),
            Value::Nil
        );
        assert_eq!(
            get_length(&[Value::Int(0)]).expect("zero handled"),
            Value::Nil
        );
    }

    #[test]
    fn get_length_errors_for_unsupported_types() {
        let error = get_length(&[Value::Int(5)]).expect_err("GetLength rejects unsupported");
        assert_eq!(
            error.message(),
            "func \"GetLength\" par 0 cannot be converted to string or array or map"
        );
    }

    #[test]
    fn get_index_of_returns_matching_index_or_negative_one() {
        let array = Value::Array(vec![
            Value::Int(5),
            Value::String("target".into()),
            Value::Int(7),
        ]);

        let found = get_index_of(&[Value::String("target".into()), array.clone()])
            .expect("GetIndexOf succeeds");
        assert_eq!(found, Value::Int(1));

        let missing =
            get_index_of(&[Value::String("missing".into()), array]).expect("missing handled");
        assert_eq!(missing, Value::Int(-1));

        let non_array =
            get_index_of(&[Value::Int(1), Value::Bool(true)]).expect("non-array handled");
        assert_eq!(non_array, Value::Int(-1));
    }

    #[test]
    fn log_message_emits_info_event_with_script_target() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(Arc::clone(&records));
        let subscriber = Registry::default().with(layer);
        subscriber::with_default(subscriber, || {
            let args = [Value::String("Log %02d".into()), Value::Int(3)];
            let result = log_message(&args).expect("Log succeeds");
            assert_eq!(result, Value::Bool(true));
        });
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.level, Level::INFO);
        assert_eq!(record.target, "lc-script");
        assert_eq!(record.message, "Log 03");
    }

    #[test]
    fn debug_log_message_emits_debug_event_with_script_target() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(Arc::clone(&records));
        let subscriber = Registry::default().with(layer);
        subscriber::with_default(subscriber, || {
            let args = [Value::String("Debug %d".into()), Value::Int(42)];
            let result = debug_log_message(&args).expect("DebugLog succeeds");
            assert_eq!(result, Value::Bool(true));
        });
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.level, Level::DEBUG);
        assert_eq!(record.target, "lc-script");
        assert_eq!(record.message, "Debug 42");
    }

    #[test]
    fn game_over_returns_true_only_once_per_context() {
        let (result, outcome) = with_effect_context_with_state(
            None,
            &[],
            HostWorldContext::default(),
            1,
            false,
            || {
                let first = game_over(&[])?;
                assert_eq!(first, Value::Bool(true));
                game_over(&[])
            },
        );
        let second = result.expect("GameOver second call succeeds");
        assert_eq!(second, Value::Bool(false));
        assert!(outcome.trigger_game_over);
    }

    #[test]
    fn game_over_respects_existing_state() {
        let (result, outcome) =
            with_effect_context_with_state(None, &[], HostWorldContext::default(), 1, true, || {
                game_over(&[])
            });
        let value = result.expect("GameOver call succeeds");
        assert_eq!(value, Value::Bool(false));
        assert!(!outcome.trigger_game_over);
    }

    #[test]
    fn g_back_solid_returns_false_without_landscape() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            g_back_solid(&[Value::Int(0), Value::Int(0)])
        });
        let value = result.expect("GBackSolid without landscape succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_semi_solid_counts_liquids_like_cpp() {
        // GBackSemiSolid = DensitySemiSolid(GBackDensity) = density >=
        // C4M_SemiSolid(25) (C4Wrappers.h:73-76, C4Material.h:202): water
        // is semi-solid but NOT solid.
        let mut landscape = Landscape::flat(32, 20);
        landscape.set_liquid_column(
            5,
            vec![crate::landscape::LiquidSegment {
                top: 10,
                bottom: 19,
                material: None,
            }],
        );
        let world = || {
            HostWorldContext::with_landscape(
                Vec::<HostWorldObject>::new(),
                Some(landscape.clone()),
                HashMap::new(),
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                1,
                false,
            )
        };
        let (semi, _) = with_effect_context(None, &[], world(), 1, || {
            g_back_semi_solid(&[Value::Int(5), Value::Int(15)])
        });
        assert_eq!(
            semi.expect("GBackSemiSolid succeeds"),
            Value::Bool(true),
            "water is semi-solid"
        );
        let (solid, _) = with_effect_context(None, &[], world(), 1, || {
            g_back_solid(&[Value::Int(5), Value::Int(15)])
        });
        assert_eq!(
            solid.expect("GBackSolid succeeds"),
            Value::Bool(false),
            "water is not solid"
        );
    }

    #[test]
    fn g_back_solid_detects_surface_in_landscape() {
        let landscape = Landscape::flat(32, 10);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(5), Value::Int(12)])
        });
        let value = result.expect("GBackSolid with landscape succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_solid_respects_surface_height() {
        let landscape = Landscape::flat(16, 20);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(3), Value::Int(15)])
        });
        let value = result.expect("GBackSolid above surface succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_solid_applies_object_relative_coordinates() {
        let object_id = ObjectId::new(7);
        let landscape = Landscape::flat(32, 12);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            8,
            false,
        );
        let object_context = HostObjectContext::new(
            object_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::new(4, 6),
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let (result, _) = with_effect_context(Some(object_context), &[], world, 9, || {
            g_back_solid(&[Value::Int(0), Value::Int(7)])
        });
        let value = result.expect("GBackSolid with object context succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_sky_reports_inverse_of_solid() {
        let landscape = Landscape::flat(20, 5);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (solid, _) = with_effect_context(None, &[], world.clone(), 1, || {
            g_back_solid(&[Value::Int(2), Value::Int(2)])
        });
        let (sky, _) = with_effect_context(None, &[], world, 1, || {
            g_back_sky(&[Value::Int(2), Value::Int(2)])
        });
        let solid_value = solid.expect("GBackSolid succeeds");
        let sky_value = sky.expect("GBackSky succeeds");
        match (solid_value, sky_value) {
            (Value::Bool(solid), Value::Bool(sky)) => assert_eq!(sky, !solid),
            other => panic!("expected bool results, got {:?}", other),
        }
    }

    #[test]
    fn get_material_returns_mnone_without_context() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            get_material(&[Value::Int(0), Value::Int(0)])
        });
        assert_eq!(
            result.expect("GetMaterial without context succeeds"),
            Value::Int(MATERIAL_NONE)
        );
    }

    #[test]
    fn get_material_reports_solid_material_from_landscape() {
        let material = crate::MaterialId::new(3).expect("material id");
        let landscape = Landscape::flat_with_material(32, 10, Some(material));
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            get_material(&[Value::Int(5), Value::Int(12)])
        });
        let expected = Value::Int(material.index() as i32);
        assert_eq!(
            result.expect("GetMaterial with landscape succeeds"),
            expected
        );
    }

    #[test]
    fn get_material_applies_object_relative_coordinates() {
        let material = crate::MaterialId::new(2).expect("material id");
        let landscape = Landscape::flat_with_material(24, 12, Some(material));
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            5,
            false,
        );
        let object_id = ObjectId::new(11);
        let object_context = HostObjectContext::new(
            object_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::new(3, 4),
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let (result, _) = with_effect_context(Some(object_context), &[], world, 6, || {
            get_material(&[Value::Int(0), Value::Int(8)])
        });
        assert_eq!(
            result.expect("GetMaterial with object succeeds"),
            Value::Int(material.index() as i32)
        );
    }

    #[test]
    fn dig_free_registers_landscape_operation() {
        let args = [
            Value::Int(42),
            Value::Int(128),
            Value::Int(6),
            Value::Bool(true),
        ];
        let (result, outcome) = with_object_host_context(|| dig_free(&args));
        assert_eq!(result.expect("DigFree succeeds"), Value::Bool(true));
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DigCircle {
                center,
                radius,
                requested,
                by_object,
            } => {
                assert_eq!(*center, Vector2::new(42, 128));
                assert_eq!(*radius, 6);
                assert!(*requested);
                assert!(by_object.is_some());
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn dig_free_rect_requires_positive_dimensions() {
        let args = [Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(4)];
        let (result, outcome) = with_object_host_context(|| dig_free_rect(&args));
        assert_eq!(result.expect("DigFreeRect succeeds"), Value::Bool(false));
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn dig_free_rect_registers_landscape_operation() {
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Int(5),
            Value::Int(7),
            Value::Bool(false),
        ];
        let (result, outcome) = with_object_host_context(|| dig_free_rect(&args));
        assert_eq!(result.expect("DigFreeRect succeeds"), Value::Bool(true));
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DigRect {
                origin,
                width,
                height,
                requested,
                by_object,
            } => {
                assert_eq!(*origin, Vector2::new(10, 20));
                assert_eq!(*width, 5);
                assert_eq!(*height, 7);
                assert!(!*requested);
                assert!(by_object.is_some());
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_registers_landscape_operation() {
        let args = [Value::Int(12), Value::Int(34), Value::Int(5), Value::Int(3)];
        let (result, outcome) = with_object_host_context(|| blast_free(&args));
        assert_eq!(result.expect("BlastFree succeeds"), Value::Bool(true));
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::BlastCircle {
                center,
                radius,
                controller,
            } => {
                assert_eq!(*center, Vector2::new(12, 34));
                assert_eq!(*radius, 5);
                assert_eq!(*controller, Some(2));
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_offsets_coordinates_without_explicit_controller() {
        let object_context = HostObjectContext::new(
            ObjectId::new(1),
            None,
            ObjectStatus::Normal,
            100,
            4,
            Vector2::new(5, 10),
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            FULL_CON,
        );
        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            1,
            || blast_free(&[Value::Int(3), Value::Int(7), Value::Int(6)]),
        );
        assert_eq!(result.expect("BlastFree succeeds"), Value::Bool(true));
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::BlastCircle {
                center,
                radius,
                controller,
            } => {
                assert_eq!(*center, Vector2::new(8, 17));
                assert_eq!(*radius, 6);
                assert_eq!(*controller, Some(4));
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_rejects_non_positive_level() {
        let args = [Value::Int(0), Value::Int(0), Value::Int(0)];
        let (result, outcome) = with_object_host_context(|| blast_free(&args));
        assert_eq!(
            result.expect("BlastFree handles zero level"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn shake_free_registers_landscape_operation() {
        let args = [Value::Int(30), Value::Int(40), Value::Int(5)];
        let (result, outcome) = with_object_host_context(|| shake_free(&args));
        assert_eq!(result.expect("ShakeFree succeeds"), Value::Bool(true));
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::ShakeCircle { center, radius } => {
                assert_eq!(*center, Vector2::new(30, 40));
                assert_eq!(*radius, 5);
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn shake_free_rejects_non_positive_radius() {
        let args = [Value::Int(10), Value::Int(20), Value::Int(0)];
        let (result, outcome) = with_object_host_context(|| shake_free(&args));
        assert_eq!(
            result.expect("ShakeFree handles zero radius"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn g_back_liquid_returns_false_in_height_landscape() {
        let landscape = Landscape::flat(8, 4);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_liquid_detects_liquid_column() {
        let mut landscape = Landscape::flat(8, 4);
        landscape.set_liquid_column(1, vec![LiquidSegment::new(5, 9)]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn get_player_count_counts_registered_players() {
        let mut alice = PlayerState::default();
        alice.id = 1;
        alice.name = "Alice".into();
        let mut bob = PlayerState::default();
        bob.id = 2;
        bob.name = "Bob".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![alice, bob],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_count(&[]));
        assert_eq!(result.expect("GetPlayerCount succeeds"), Value::Int(2));
    }

    #[test]
    fn get_player_by_index_returns_player_number() {
        let mut alice = PlayerState::default();
        alice.id = 1;
        alice.name = "Alice".into();
        let mut carol = PlayerState::default();
        carol.id = 3;
        carol.name = "Carol".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![alice, carol],
        );
        let args = [Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_by_index(&args));
        assert_eq!(result.expect("GetPlayerByIndex succeeds"), Value::Int(3));
    }

    #[test]
    fn get_player_name_returns_registered_name() {
        let mut player = PlayerState::default();
        player.id = 5;
        player.name = "Delta".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(5)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_name(&args));
        assert_eq!(
            result.expect("GetPlayerName succeeds"),
            Value::String("Delta".into())
        );
    }

    #[test]
    fn get_player_team_returns_nil_when_unset() {
        let player = PlayerState {
            id: 7,
            name: "Eta".into(),
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(7)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_team(&args));
        assert_eq!(result.expect("GetPlayerTeam succeeds"), Value::Nil);
    }

    #[test]
    fn get_wealth_returns_player_wealth() {
        let player = PlayerState {
            id: 12,
            wealth: 87,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(12)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_wealth(&args));
        assert_eq!(result.expect("GetWealth succeeds"), Value::Int(87));
    }

    #[test]
    fn set_wealth_clamps_and_records_player_command() {
        // FnSetWealth (C4Script.cpp:2761-2766): clamp-set to 0..=100000,
        // false for invalid players. (DoWealth's 10000 cap applies only to
        // the engine-internal adjust path, C4Player.cpp:905-915.)
        let player = PlayerState {
            id: 12,
            wealth: 87,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                set_wealth(&[Value::Int(12), Value::Int(150_000)])?,
                Value::Bool(true)
            );
            // The same callback observes the clamped value.
            assert_eq!(get_wealth(&[Value::Int(12)])?, Value::Int(100_000));
            // Invalid player (C4Script.cpp:2763).
            assert_eq!(
                set_wealth(&[Value::Int(5), Value::Int(10)])?,
                Value::Bool(false)
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("SetWealth succeeds");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetWealth {
                player_id: 12,
                value: 100_000,
            }]
        ));
    }

    #[test]
    fn get_score_returns_player_points() {
        let player = PlayerState {
            id: 4,
            points: 135,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(4)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_score(&args));
        assert_eq!(result.expect("GetScore succeeds"), Value::Int(135));
    }

    #[test]
    fn get_plr_value_returns_total_value() {
        let player = PlayerState {
            id: 9,
            value: 320,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(9)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_value(&args));
        assert_eq!(result.expect("GetPlrValue succeeds"), Value::Int(320));
    }

    #[test]
    fn get_plr_value_gain_returns_gain() {
        let player = PlayerState {
            id: 9,
            value_gain: 45,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(9)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_value_gain(&args));
        assert_eq!(result.expect("GetPlrValueGain succeeds"), Value::Int(45));
    }

    #[test]
    fn get_plr_knowledge_reports_known_definition() {
        let mut player = PlayerState::default();
        player.id = 5;
        player.knowledge = vec!["BRIK".to_string()];
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(5, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(5), Value::String("BRIK".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_knowledge(&args));

        assert_eq!(result.expect("GetPlrKnowledge succeeds"), Value::Bool(true));
    }

    #[test]
    fn get_plr_knowledge_returns_definition_by_index() {
        let mut player = PlayerState::default();
        player.id = 6;
        player.knowledge = vec!["BRIK".to_string(), "STON".to_string()];
        let definitions = HashMap::from([
            (
                "BRIK".to_string(),
                DefinitionMetadata {
                    name: String::new(),
                    category: 0x1,
                    ocf_base: 0,
                    crew_member: false,
                    action_library: ActionLibrary::default(),
                    value: 0,
                    mass: 0,
                    constructable: false,
                    shape: None,
                    construction_offset: 0,
                    basement: 0,
                    physical: PhysicalInfo::default(),
                    components: Vec::new(),
                    line_connect: 0,
                    clonk_name_newlines: None,
                    stretch_growth: false,
                    line: 0,
                    vertices: Vec::new(),
                },
            ),
            (
                "STON".to_string(),
                DefinitionMetadata {
                    name: String::new(),
                    category: 0x2,
                    ocf_base: 0,
                    crew_member: false,
                    action_library: ActionLibrary::default(),
                    value: 0,
                    mass: 0,
                    constructable: false,
                    shape: None,
                    construction_offset: 0,
                    basement: 0,
                    physical: PhysicalInfo::default(),
                    components: Vec::new(),
                    line_connect: 0,
                    clonk_name_newlines: None,
                    stretch_growth: false,
                    line: 0,
                    vertices: Vec::new(),
                },
            ),
        ]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(6, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(6), Value::Nil, Value::Int(0), Value::Int(0x2)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_knowledge(&args));

        assert_eq!(
            result.expect("GetPlrKnowledge succeeds"),
            Value::String("STON".into())
        );
    }

    #[test]
    fn set_plr_knowledge_grants_definition_and_records_command() {
        let mut player = PlayerState::default();
        player.id = 7;
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(7, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::Int(7),
            Value::String("BRIK".into()),
            Value::Bool(false),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || set_plr_knowledge(&args));

        assert_eq!(result.expect("SetPlrKnowledge succeeds"), Value::Bool(true));
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::GrantKnowledge {
                player_id,
                definition_id,
            } => {
                assert_eq!(*player_id, 7);
                assert_eq!(definition_id, "BRIK");
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn set_plr_knowledge_revokes_definition_and_records_command() {
        let mut player = PlayerState::default();
        player.id = 8;
        player.knowledge = vec!["BRIK".to_string()];
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(8, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::Int(8),
            Value::String("BRIK".into()),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || set_plr_knowledge(&args));

        assert_eq!(result.expect("SetPlrKnowledge succeeds"), Value::Bool(true));
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::RevokeKnowledge {
                player_id,
                definition_id,
            } => {
                assert_eq!(*player_id, 8);
                assert_eq!(definition_id, "BRIK");
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn definition_arguments_accept_c4id_values_like_cpp() {
        // FnFindObject and friends take a C4ID-typed first parameter
        // (C4Script.cpp FindObject: C4ID id): definition constants reach
        // host functions as C4Id values and must resolve — GoldRush's
        // DoInitialize probes `FindObject(NOPC)` with them.
        let parsed = parse_definition_argument(Some(&Value::C4Id("NOPC".into())), "FindObject")
            .expect("C4Id accepted");
        assert_eq!(parsed.as_deref(), Some("NOPC"));
    }

    #[test]
    fn get_component_answers_def_counts_and_indexed_ids() {
        // FnGetComponent (C4Script.cpp:2685-2709): with idDef the def's
        // component list answers; idComponent selects the count form,
        // otherwise the index form returns the id (C4VID).
        let mut metadata = DefinitionMetadata {
            name: String::new(),
            category: 0,
            ocf_base: 0,
            crew_member: false,
            action_library: ActionLibrary::default(),
            value: 0,
            mass: 0,
            constructable: false,
            shape: None,
            construction_offset: 0,
            basement: 0,
            physical: lc_resources::PhysicalInfo::default(),
            components: Vec::new(),
            line_connect: 0,
            clonk_name_newlines: None,
            stretch_growth: false,
            line: 0,
            vertices: Vec::new(),
        };
        metadata.components = vec![("WOOD".to_string(), 3), ("METL".to_string(), 1)];
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::from([(DefinitionId::from("HUTT"), metadata)]),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world.clone(), 1, || {
            let count = get_component(&[
                Value::C4Id("WOOD".into()),
                Value::Int(0),
                Value::Nil,
                Value::C4Id("HUTT".into()),
            ])?;
            assert_eq!(count, Value::Int(3), "count form");
            get_component(&[
                Value::Nil,
                Value::Int(1),
                Value::Nil,
                Value::C4Id("HUTT".into()),
            ])
        });
        assert_eq!(
            result.expect("GetComponent succeeds"),
            Value::C4Id("METL".into()),
            "index form returns the id"
        );
    }

    #[test]
    fn material_resolves_names_to_numbers_like_cpp() {
        // FnMaterial (C4Script.cpp:2488-2491): Game.Material.Get — the
        // material number, -1 for unknown names.
        let library =
            lc_resources::MaterialLibrary::parse("[Material Earth]\nName=Earth\nDensity=50\n")
                .expect("library builds");
        let materials = MaterialSet::from_resource_library(&library);
        let expected = materials.get("Earth").expect("earth exists").id().index() as i32;
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
        .with_materials(Some(Rc::new(materials)));
        let (result, _) = with_effect_context(None, &[], world.clone(), 1, || {
            let known = material(&[Value::String("Earth".into())])?;
            assert_eq!(known, Value::Int(expected));
            material(&[Value::String("Unobtainium".into())])
        });
        assert_eq!(
            result.expect("Material succeeds"),
            Value::Int(MATERIAL_NONE)
        );
    }

    #[test]
    fn get_hi_rank_prefers_higher_rank_then_crew_order() {
        // FnGetHiRank (C4Script.cpp:2792-2796) ->
        // C4Player::GetHiRankActiveCrew(false) (C4Player.cpp:1003-1020):
        // walk the crew in order, rank from the linked Info (no info =
        // -1); only a STRICTLY higher rank replaces, so the first of equal
        // ranks wins.
        let crew_ids = [11_u64, 22_u64, 33_u64];
        let objects: Vec<HostWorldObject> = crew_ids
            .iter()
            .map(|&id| {
                HostWorldObject::new(
                    ObjectId::new(id),
                    "Clonk",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    1,
                    100,
                    crate::FULL_CON,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                )
            })
            .collect();
        let mut player = PlayerState::default();
        player.id = 1;
        player.crew = crew_ids.iter().map(|&id| ObjectId::new(id)).collect();

        let world = HostWorldContext::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(1, player)]),
            HashMap::new(),
            1,
            false,
        )
        .with_crew_ranks(std::rc::Rc::new(HashMap::from([
            (11_u64, 0),
            (22_u64, 3),
            (33_u64, 3),
        ])));
        let (result, _) =
            with_effect_context(None, &[], world, 1, || get_hi_rank(&[Value::Int(1)]));
        assert_eq!(
            result.expect("GetHiRank succeeds"),
            object_reference_value(ObjectId::new(22)),
            "rank 3 beats rank 0; the FIRST rank-3 member wins the tie"
        );
    }

    #[test]
    fn get_crew_returns_nth_crew_member() {
        let crew_ids = [101_u64, 202_u64];
        let objects = vec![
            HostWorldObject::new(
                ObjectId::new(crew_ids[0]),
                "Clonk",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(crew_ids[1]),
                "Clonk",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ];
        let mut player = PlayerState::default();
        player.id = 1;
        player.crew = vec![ObjectId::new(crew_ids[0]), ObjectId::new(crew_ids[1])];

        let world = HostWorldContext::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(1, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(1), Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew(&args));

        assert_eq!(
            result.expect("GetCrew succeeds"),
            object_reference_value(ObjectId::new(crew_ids[1]))
        );
    }

    #[test]
    fn get_crew_returns_nil_for_out_of_range_index() {
        let crew_ids = [700_u64];
        let objects = vec![HostWorldObject::new(
            ObjectId::new(crew_ids[0]),
            "Clonk",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            3,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )];
        let mut player = PlayerState::default();
        player.id = 3;
        player.crew = vec![ObjectId::new(crew_ids[0])];

        let world = HostWorldContext::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(3, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(3), Value::Int(5)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew(&args));

        assert_eq!(result.expect("GetCrew succeeds"), Value::Nil);
    }

    #[test]
    fn get_crew_count_reports_total_crew() {
        let crew_ids = [303_u64, 404_u64, 505_u64];
        let objects = crew_ids
            .iter()
            .map(|id| {
                HostWorldObject::new(
                    ObjectId::new(*id),
                    "Clonk",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    2,
                    100,
                    crate::FULL_CON,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                )
            })
            .collect::<Vec<_>>();
        let mut player = PlayerState::default();
        player.id = 2;
        player.crew = crew_ids
            .iter()
            .map(|id| ObjectId::new(*id))
            .collect::<Vec<_>>();

        let world = HostWorldContext::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(2, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(2)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew_count(&args));

        assert_eq!(result.expect("GetCrewCount succeeds"), Value::Int(3));
    }

    #[test]
    fn get_cursor_defaults_to_current_cursor() {
        let cursor = ObjectId::new(900);
        let mut player = PlayerState::default();
        player.id = 12;
        player.cursor = Some(cursor);
        player.crew = vec![cursor];
        let selection = CrewSelectionState {
            selected: vec![cursor],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(12, player)]),
            HashMap::from([(12, selection)]),
            1,
            false,
        );
        let args = [Value::Int(12)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_cursor_host(&args));

        assert_eq!(
            result.expect("GetCursor succeeds"),
            object_reference_value(cursor)
        );
    }

    #[test]
    fn get_cursor_returns_selected_member_by_index() {
        let cursor = ObjectId::new(910);
        let other = ObjectId::new(920);
        let mut player = PlayerState::default();
        player.id = 13;
        player.cursor = Some(cursor);
        player.crew = vec![cursor, other];
        let selection = CrewSelectionState {
            selected: vec![cursor, other],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(13, player)]),
            HashMap::from([(13, selection)]),
            1,
            false,
        );
        let args = [Value::Int(13), Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_cursor_host(&args));

        assert_eq!(
            result.expect("GetCursor succeeds"),
            object_reference_value(other)
        );
    }

    #[test]
    fn get_select_count_reports_selected_units() {
        let cursor = ObjectId::new(930);
        let other = ObjectId::new(940);
        let mut player = PlayerState::default();
        player.id = 14;
        player.cursor = Some(cursor);
        player.crew = vec![cursor, other];
        let selection = CrewSelectionState {
            selected: vec![cursor, other],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(14, player)]),
            HashMap::from([(14, selection)]),
            1,
            false,
        );
        let args = [Value::Int(14)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_select_count(&args));

        assert_eq!(result.expect("GetSelectCount succeeds"), Value::Int(2));
    }

    #[test]
    fn get_view_cursor_returns_first_focus_target() {
        let focus = ObjectId::new(950);
        let mut player = PlayerState::default();
        player.id = 15;
        player
            .viewports
            .push(PlayerViewport::new(Vector2::ZERO).with_focus(Some(focus)));
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(15, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(15)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_view_cursor(&args));

        assert_eq!(
            result.expect("GetViewCursor succeeds"),
            object_reference_value(focus)
        );
    }

    #[test]
    fn get_homebase_material_returns_count_for_definition() {
        let mut player = PlayerState::default();
        player.id = 1;
        player.home_base_material.insert("Brick".to_string(), 3_u32);
        let definitions = HashMap::from([(
            "Brick".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(1, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(1), Value::String("Brick".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_homebase_material(&args));

        assert_eq!(result.expect("GetHomebaseMaterial succeeds"), Value::Int(3));
    }

    #[test]
    fn do_homebase_material_records_player_command() {
        let mut player = PlayerState::default();
        player.id = 1;
        player.home_base_material.insert("Brick".to_string(), 1_u32);
        let definitions = HashMap::from([(
            "Brick".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(1, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(1), Value::String("Brick".into()), Value::Int(2)];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || do_homebase_material(&args));

        assert_eq!(
            result.expect("DoHomebaseMaterial succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::AdjustHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 1);
                assert_eq!(definition_id, "Brick");
                assert_eq!(*delta, 2);
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn do_homebase_production_records_player_command() {
        let mut player = PlayerState::default();
        player.id = 1;
        let definitions = HashMap::from([(
            "Brick".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: 1,
                ocf_base: 0,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            None,
            definitions,
            Vec::new(),
            HashMap::from([(1, player)]),
            HashMap::new(),
            1,
            false,
        );
        let args = [Value::Int(1), Value::String("Brick".into()), Value::Int(1)];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || do_homebase_production(&args));

        assert_eq!(
            result.expect("DoHomebaseProduction succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::AdjustHomeBaseProduction {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 1);
                assert_eq!(definition_id, "Brick");
                assert_eq!(*delta, 1);
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn set_transfer_zone_registers_command_for_active_object() {
        let args = [Value::Int(2), Value::Int(3), Value::Int(5), Value::Int(7)];
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                ObjectId::new(1),
                "ZoneTester",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, outcome) =
            with_object_host_context_with_world(world, || set_transfer_zone(&args));
        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match &outcome.transfer_zones[0] {
            TransferZoneCommand::Set { owner, rect } => {
                assert_eq!(*owner, ObjectId::new(1));
                assert_eq!(rect.x, 2);
                assert_eq!(rect.y, 3);
                assert_eq!(rect.width, 5);
                assert_eq!(rect.height, 7);
            }
            other => panic!("expected set command, got {:?}", other),
        }
    }

    #[test]
    fn set_transfer_zone_with_zero_size_clears_existing() {
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                ObjectId::new(1),
                "ZoneTester",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_transfer_zone(&[Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(10)])
        });
        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match outcome.transfer_zones.first() {
            Some(TransferZoneCommand::Clear { owner }) => {
                assert_eq!(*owner, ObjectId::new(1));
            }
            other => panic!("expected clear command, got {:?}", other),
        }
    }

    #[test]
    fn add_effect_registers_command_and_updates_view() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(150),
                Value::Int(3),
            ])
        });
        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 150);
                assert_eq!(effect.interval, 3);
                assert_eq!(effect.command_target, None);
                assert!(effect.command_id.is_none());
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn add_effect_records_command_target_metadata() {
        let state = empty_state();
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(42));
        let target = Value::Proplist(target_map);

        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                target.clone(),
                Value::String("FOOB".into()),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.command_target, Some(42));
                assert_eq!(effect.command_id.as_deref(), Some("FOOB"));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn set_gravity_records_physics_update() {
        let (result, delta) =
            with_physics_context(PhysicsSettings::default(), || set_gravity(&[Value::Int(5)]));
        let value = result.expect("SetGravity succeeds");
        assert_eq!(value, Value::Nil);
        assert_eq!(delta.gravity, Some(5));
    }

    #[test]
    fn set_gravity_clamps_bounds() {
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(400)])
        });
        assert_eq!(delta.gravity, Some(300));
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(-500)])
        });
        assert_eq!(delta.gravity, Some(-300));
    }

    #[test]
    fn get_gravity_returns_current_value() {
        let settings = PhysicsSettings::new(6, 20, -30);
        let (result, _) = with_physics_context(settings, || get_gravity(&[]));
        let value = result.expect("GetGravity succeeds");
        assert_eq!(value, Value::Int(6));
    }

    #[test]
    fn set_wind_records_environment_update() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_wind(&[Value::Int(75)])?;
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(75));
        assert_eq!(delta.wind, Some(75));
    }

    #[test]
    fn get_wind_positional_reads_tunnel_background() {
        // FnGetWind (C4Script.cpp:3001-3008): the global form returns
        // Weather.Wind; the positional form reads GBackWind — zero inside
        // tunnel-background (IFT) pixels (C4Wrappers.h:189-192).
        let mut landscape = Landscape::flat(32, 100);
        landscape.set_tunnel_column(5, vec![(0, 20)]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_environment_context(EnvironmentSettings::new(60), 0, || {
            let (inner, _) = with_effect_context(None, &[], world, 1, || {
                assert_eq!(get_wind(&[Value::Int(5), Value::Int(10)])?, Value::Int(0));
                assert_eq!(get_wind(&[Value::Int(6), Value::Int(10)])?, Value::Int(60));
                assert_eq!(
                    get_wind(&[Value::Nil, Value::Nil, Value::Bool(true)])?,
                    Value::Int(60)
                );
                Ok(Value::Nil)
            });
            inner
        });
        result.expect("GetWind positional succeeds");
    }

    #[test]
    fn set_wind_clamps_to_bounds() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_wind(&[Value::Int(150)])?;
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(100));
        assert_eq!(delta.wind, Some(100));
    }

    #[test]
    fn set_temperature_updates_context() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 42, || {
            set_temperature(&[Value::Int(-30)])?;
            get_temperature(&[])
        });

        let value = result.expect("SetTemperature/GetTemperature succeeds");
        assert_eq!(value, Value::Int(-30));
        assert_eq!(delta.temperature, Some(-30));
    }

    #[test]
    fn set_climate_clamps_and_updates() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_climate(&[Value::Int(-80)])?;
            get_climate(&[])
        });

        let value = result.expect("SetClimate/GetClimate succeeds");
        assert_eq!(value, Value::Int(-50));
        assert_eq!(delta.climate, Some(-50));
    }

    #[test]
    fn random_requires_context_for_positive_ranges() {
        let error = random(&[Value::Int(5)]).expect_err("Random without context fails");
        assert_eq!(error.message(), "Random: host context unavailable");
    }

    #[test]
    fn random_edge_ranges_follow_the_cpp_ledger() {
        // C4Random.h:40-61: RandomCount++ is UNCONDITIONAL; range 0
        // returns 0 without advancing the hold; nil converts to 0 at the
        // host boundary (C4AulExec.cpp:1364-1396); a negative range goes
        // through the unsigned modulo (usual arithmetic conversions), so
        // the hold DOES advance.
        let guard = enter_random_context(LcgRng::new(0));
        let zero = random(&[Value::Int(0)]).expect("zero range succeeds");
        assert_eq!(zero, Value::Int(0));
        let nil = random(&[Value::Nil]).expect("nil converts to 0");
        assert_eq!(nil, Value::Int(0));
        let missing = random(&[]).expect("missing argument converts to 0");
        assert_eq!(missing, Value::Int(0));
        let negative = random(&[Value::Int(-3)]).expect("negative range succeeds");
        let rng = guard.finish();
        // Three zero-ish draws (count++ only) plus one negative draw that
        // advances the hold like C++'s unsigned modulo.
        assert_eq!(rng.count, 4, "RandomCount++ is unconditional");
        let mut reference = LcgRng::new(0);
        reference.random(0);
        reference.random(0);
        reference.random(0);
        assert_eq!(Value::Int(reference.random(-3)), negative);
        assert_eq!(rng.hold, reference.hold, "negative ranges advance the hold");
    }

    proptest! {
        #[test]
        fn set_wind_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_wind(&[Value::Int(raw)])?;
                get_wind(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.wind, Some(expected));
            prop_assert!(delta.temperature.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_temperature_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_temperature(&[Value::Int(raw)])?;
                get_temperature(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.temperature, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_climate_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_climate(&[Value::Int(raw)])?;
                get_climate(&[])
            });

            let expected = raw.clamp(-50, 50);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.climate, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.temperature.is_none());
        }

        #[test]
        fn random_matches_cpp_lcg(seed in any::<u64>(), range in 1i32..=1024) {
            let mut expected_rng = LcgRng::new(seed as u32);
            let expected = expected_rng.random(range);

            let guard = enter_random_context(LcgRng::new(seed as u32));
            let value = random(&[Value::Int(range)]).expect("Random with context succeeds");
            let _ = guard.finish();

            prop_assert_eq!(value, Value::Int(expected));
            prop_assert!(expected >= 0 && expected < range);
        }

        #[test]
        fn random_sequence_remains_deterministic(seed in any::<u64>()) {
            let mut expected_rng = LcgRng::new(seed as u32);
            let expected = [
                expected_rng.random(100),
                expected_rng.random(100),
                expected_rng.random(100),
            ];

            let guard = enter_random_context(LcgRng::new(seed as u32));
            let first = random(&[Value::Int(100)]).expect("first draw succeeds");
            let second = random(&[Value::Int(100)]).expect("second draw succeeds");
            let third = random(&[Value::Int(100)]).expect("third draw succeeds");
            let _ = guard.finish();

            prop_assert_eq!(first, Value::Int(expected[0]));
            prop_assert_eq!(second, Value::Int(expected[1]));
            prop_assert_eq!(third, Value::Int(expected[2]));
        }
    }

    #[test]
    fn add_effect_captures_initial_vars() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(7),
                Value::Bool(true),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.vars().len(), 2);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(7));
                assert_eq!(effect.vars()[1], EffectVarValue::Bool(true));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn remove_effect_rejects_when_missing() {
        let state = empty_state();
        let (result, _) =
            with_object_host_context(|| remove_effect(&[Value::Nil, state.clone(), Value::Int(0)]));
        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn add_and_remove_effect_flow() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            remove_effect(&[Value::String("Glow".into()), state.clone()])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add(_)));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::Remove {
                no_callbacks: false,
                ..
            }
        ));
    }

    #[test]
    fn remove_effect_can_skip_callbacks() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            remove_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Bool(true),
            ])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add(_)));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::Remove {
                no_callbacks: true,
                ..
            }
        ));
    }

    #[test]
    fn set_action_respects_no_other_action() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library.clone(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Walk".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(2),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Idle".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("pending update exists");
        let action = update.action.expect("action update recorded");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert!(!action.force);
    }

    #[test]
    fn set_action_data_records_object_update() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(512)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update present");
        assert_eq!(action.data, Some(255));
    }

    #[test]
    fn set_action_data_rejects_invalid_attach_vertices() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("attach"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(31 << 8)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_action_data_requires_active_object() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Inactive,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(5)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_data_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action_data(&[]));
        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_data_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action_data(&[Value::Int(42)])?;
                get_action_data(&[])
            },
        );

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(42));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.data, Some(42));
    }

    #[test]
    fn get_action_data_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(23),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            77,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(23));
            get_action_data(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(77));
    }

    #[test]
    fn get_action_data_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action_data(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_data_returns_nil_without_context() {
        let value = get_action_data(&[]).expect("GetActionData succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_returns_idle_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action(&[]));
        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Idle".into()));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_reflects_pending_update() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_action(&[])
            },
        );

        let value = result.expect("SetAction/GetAction succeed");
        assert_eq!(value, Value::String("Walk".into()));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.name.as_deref(), Some("Walk"));
    }

    #[test]
    fn get_procedure_returns_nil_when_unspecified() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_procedure_returns_configured_value() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("walk".into()));
    }

    #[test]
    fn get_procedure_reflects_pending_action_change() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        specs.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Float".into())])?;
                get_procedure(&[])
            },
        );

        let value = result.expect("SetAction/GetProcedure succeed");
        assert_eq!(value, Value::String("float".into()));
    }

    #[test]
    fn get_procedure_reads_world_context() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(42),
            "Dummy",
            ObjectStatus::Normal,
            "Swim",
            None,
            None,
            Some("swim".to_string()),
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(42));
            get_procedure(&[Value::Proplist(target)])
        });

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("swim".into()));
    }

    #[test]
    fn get_action_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            let target = Value::Proplist(target);
            get_action(&[target])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_reads_other_object_from_world() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action(&[Value::Proplist(target)])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Walk".into()));
    }

    #[test]
    fn get_action_uses_world_without_context() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Dig",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(7));
            get_action(&[Value::Proplist(target)])
        });

        let value = result.expect("GetAction resolves world lookup");
        assert_eq!(value, Value::String("Dig".into()));
    }

    #[test]
    fn get_action_returns_nil_without_context() {
        let value = get_action(&[]).expect("GetAction succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_act_time_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_act_time(&[]));
        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_act_time_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                7,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                // Re-setting the SAME action keeps the running ticks (only
                // an action CHANGE resets them); C++ SetAction has no
                // ticks parameter (C4Script.cpp:747-753).
                set_action(&[Value::String("Idle".into())])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(7));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(
            action.ticks, None,
            "no ticks write: the running value stands"
        );
    }

    #[test]
    fn get_act_time_resets_on_action_change() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                5,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(0));
    }

    #[test]
    fn get_act_time_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(23),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            12,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(23));
            get_act_time(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(12));
    }

    #[test]
    fn get_act_time_returns_nil_without_context() {
        let value = get_act_time(&[]).expect("GetActTime succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_vertex_num_counts_vertices() {
        let vertices = [ObjectVertex::new(0, 0), ObjectVertex::new(1, -1)];
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex_num(&[]),
        );

        let value = result.expect("GetVertexNum succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn get_vertex_returns_requested_attributes() {
        let vertex = ObjectVertex::new(2, -3)
            .with_cnat(CNAT_CENTER | CNAT_BOTTOM)
            .with_friction(7);
        let vertices = [vertex];
        let (x, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(0)]),
        );
        assert_eq!(x.expect("x succeeds"), Value::Int(2));
        let (y, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(1)]),
        );
        assert_eq!(y.expect("y succeeds"), Value::Int(-3));
        let (cnat, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(2)]),
        );
        assert_eq!(
            cnat.expect("cnat succeeds"),
            Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32)
        );
        let (friction, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(3)]),
        );
        assert_eq!(friction.expect("friction succeeds"), Value::Int(7));
    }

    #[test]
    fn get_vertex_contact_uses_landscape_sampling() {
        let vertices = [ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM)];
        let landscape = Landscape::flat(8, 0);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            world,
            1,
            || get_vertex_contact(&[Value::Int(0)]),
        );

        let value = result.expect("GetVertexContact succeeds");
        assert_eq!(value, Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32));
    }

    #[test]
    fn get_contact_aggregates_vertices() {
        let vertices = [
            ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM),
            ObjectVertex::new(0, -5).with_cnat(CNAT_TOP),
        ];
        let landscape = Landscape::flat(4, 0);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &vertices,
                crate::FULL_CON,
            )),
            &[],
            world,
            1,
            || get_contact(&[Value::Nil, Value::Int(-1)]),
        );

        let value = result.expect("GetContact succeeds");
        assert_eq!(value, Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32));
    }

    #[test]
    fn set_action_targets_records_target_updates() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(42));

        let (result, outcome) =
            with_object_host_context(|| set_action_targets(&[Value::Proplist(target_map.clone())]));

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(
            action.target,
            Some(Some(ObjectId::new(42))),
            "target update recorded",
        );
        assert!(action.target2.is_none(), "second target untouched");
    }

    #[test]
    fn set_action_targets_updates_second_slot_when_provided() {
        let mut first = HashMap::new();
        first.insert("id".into(), Value::Int(5));
        let mut second = HashMap::new();
        second.insert("id".into(), Value::Int(6));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[
                Value::Proplist(first.clone()),
                Value::Proplist(second.clone()),
            ])
        });

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(5))));
        assert_eq!(action.target2, Some(Some(ObjectId::new(6))));
    }

    #[test]
    fn get_action_target_reflects_pending_update() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(12));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[Value::Proplist(target_map.clone())])?;
            get_action_target(&[Value::Int(0)])
        });

        let value = result.expect("GetActionTarget succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(12)));

        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(12))));
    }

    #[test]
    fn get_action_target_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            Some(ObjectId::new(77)),
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            get_action_target(&[Value::Int(0), object_reference_value(ObjectId::new(99))])
        });

        let value = result.expect("GetActionTarget succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(77)));
    }

    #[test]
    fn get_action_target_returns_nil_for_out_of_range_index() {
        let value = with_object_host_context(|| get_action_target(&[Value::Int(2)]))
            .0
            .expect("GetActionTarget succeeds");
        assert_eq!(value, Value::Nil);
    }

    fn with_walking_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        // A two-direction active action: SetDir is gated on
        // `Action.Act > ActIdle` and `Inside(iDir, 0, Directions-1)`
        // (C4Object.cpp:4228-4230).
        let mut specs = HashMap::new();
        specs.insert(
            "Walk".to_string(),
            crate::action::ActionSpec::default().with_directions(2),
        );
        let library = ActionLibrary::new(Some("Walk".to_string()), specs);
        with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Walk",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            func,
        )
    }

    #[test]
    fn set_dir_records_direction_update() {
        let (result, outcome) = with_walking_host_context(|| set_dir(&[Value::Int(1)]));
        let value = result.expect("SetDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_phase_clamps_to_action_length_inclusive_like_cpp() {
        // C4Object::SetPhase (C4Object.cpp:2205-2211): idle → false;
        // BoundBy(iPhase, 0, Length) — INCLUSIVE of Length.
        let mut specs = HashMap::new();
        specs.insert(
            "Walk".to_string(),
            crate::action::ActionSpec::default().with_length(5),
        );
        let library = ActionLibrary::new(Some("Walk".to_string()), specs);
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Walk",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_phase(&[Value::Int(9)]),
        );
        assert_eq!(result.expect("SetPhase runs"), Value::Bool(true));
        let update = outcome.object_update.expect("phase update recorded");
        let action = update.action.expect("action update present");
        assert_eq!(action.phase, Some(5), "9 clamps to Length (5), inclusive");
    }

    #[test]
    fn set_dir_is_a_no_op_on_idle_objects() {
        // C4Object::SetDir bails on `Action.Act <= ActIdle`
        // (C4Object.cpp:4228).
        let (result, outcome) = with_object_host_context(|| set_dir(&[Value::Int(1)]));
        assert_eq!(result.expect("SetDir runs"), Value::Bool(false));
        assert!(outcome
            .object_update
            .map(|update| update.direction.is_none())
            .unwrap_or(true));
    }

    #[test]
    fn get_dir_observes_effective_direction() {
        let (result, outcome) = with_walking_host_context(|| {
            set_dir(&[Value::Int(1)])?;
            get_dir(&[])
        });
        let value = result.expect("GetDir succeeds");
        assert_eq!(value, Value::Int(Direction::Right.to_script_value()));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_com_dir_records_command_direction_update() {
        let (result, outcome) = with_object_host_context(|| set_com_dir(&[Value::Int(3)]));
        let value = result.expect("SetComDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::Right));
    }

    #[test]
    fn get_com_dir_observes_effective_command_direction() {
        let (result, outcome) = with_object_host_context(|| {
            set_com_dir(&[Value::Int(4)])?;
            get_com_dir(&[])
        });
        let value = result.expect("GetComDir succeeds");
        assert_eq!(
            value,
            Value::Int(CommandDirection::DownRight.to_script_value())
        );
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::DownRight));
    }

    #[test]
    fn set_command_clears_stack_and_pushes_command() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(10),
            Value::Int(15),
        ];
        let (result, outcome) = with_object_host_context(|| set_command(&args));
        let value = result.expect("SetCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 2);
        match &outcome.command_operations[0] {
            CommandOperation::Clear => {}
            other => panic!("expected Clear operation, got {:?}", other),
        }
        match &outcome.command_operations[1] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(10));
                assert_eq!(request.ty, Some(15));
            }
            other => panic!("expected PushFront operation, got {:?}", other),
        }
    }

    #[test]
    fn add_command_pushes_front_without_clearing() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(5),
            Value::Int(8),
        ];
        let (result, outcome) = with_object_host_context(|| add_command(&args));
        let value = result.expect("AddCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 1);
        match &outcome.command_operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(5));
                assert_eq!(request.ty, Some(8));
            }
            other => panic!("expected PushFront operation, got {:?}", other),
        }
    }

    #[test]
    fn append_command_pushes_back() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(3),
            Value::Int(4),
        ];
        let (result, outcome) = with_object_host_context(|| append_command(&args));
        let value = result.expect("AppendCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 1);
        match &outcome.command_operations[0] {
            CommandOperation::PushBack(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(3));
                assert_eq!(request.ty, Some(4));
            }
            other => panic!("expected PushBack operation, got {:?}", other),
        }
    }

    #[test]
    fn get_x_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(42, -7),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_x(&[]),
        );

        let value = result.expect("GetX succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn get_y_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(2),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(-5, 63),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_y(&[]),
        );

        let value = result.expect("GetY succeeds");
        assert_eq!(value, Value::Int(63));
    }

    #[test]
    fn get_x_reads_world_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::new(-12, 34),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(99))];

        let (result, _) = with_effect_context(None, &[], world, 1, || get_x(&args));
        let value = result.expect("GetX target succeeds");
        assert_eq!(value, Value::Int(-12));
    }

    #[test]
    fn get_y_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(1234))];
        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || get_y(&args));
        let value = result.expect("GetY handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn object_distance_defaults_to_context_object() {
        let context_id = ObjectId::new(1);
        let other_id = ObjectId::new(2);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                context_id,
                "Clonk",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(10, 15),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                other_id,
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(25, 30),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [object_reference_value(other_id)];
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                context_id,
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(10, 15),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            world,
            3,
            || object_distance(&args),
        );
        let value = result.expect("ObjectDistance succeeds");
        assert_eq!(value, Value::Int(integer_distance(10, 15, 25, 30)));
    }

    #[test]
    fn object_distance_accepts_explicit_anchor_without_host_object() {
        let anchor_id = ObjectId::new(5);
        let other_id = ObjectId::new(6);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                anchor_id,
                "Anchor",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(-40, 12),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                other_id,
                "Target",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(-10, -18),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            object_reference_value(other_id),
            object_reference_value(anchor_id),
        ];
        let (result, _) = with_effect_context(None, &[], world, 10, || object_distance(&args));
        let value = result.expect("ObjectDistance with explicit anchor succeeds");
        assert_eq!(value, Value::Int(integer_distance(-40, 12, -10, -18)));
    }

    #[test]
    fn object_distance_returns_nil_when_other_missing() {
        let args = [object_reference_value(ObjectId::new(99))];
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(3),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(0, 0),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            4,
            || object_distance(&args),
        );
        let value = result.expect("ObjectDistance with missing other succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_x_dir_returns_object_velocity() {
        let context = HostObjectContext::new(
            ObjectId::new(7),
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(12, -3),
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_x_dir(&[])
            });
        let value = result.expect("GetXDir succeeds");
        // C++ GetXDir default precision 10 returns fixtoi(xdir, 10): for a
        // 12 px/frame velocity that is 12 * 10 = 120. `C4Script.cpp:1167`.
        assert_eq!(value, Value::Int(120));
    }

    #[test]
    fn get_y_dir_applies_precision_scaling() {
        let context = HostObjectContext::new(
            ObjectId::new(8),
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(0, 25),
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let args = [Value::Nil, Value::Int(5)];
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_y_dir(&args)
            });
        let value = result.expect("GetYDir succeeds");
        // C++ GetYDir(precision = 5) returns fixtoi(ydir, 5): for a 25 px/frame
        // velocity that is 25 * 5 = 125. `C4Script.cpp:1174`.
        assert_eq!(value, Value::Int(125));
    }

    #[test]
    fn get_x_dir_reads_world_velocity_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(42),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::new(-8, 3),
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(42))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_x_dir(&args));
        let value = result.expect("GetXDir target succeeds");
        // GetXDir on another object: fixtoi(xdir, 10) for -8 px/frame = -80.
        assert_eq!(value, Value::Int(-80));
    }

    #[test]
    fn get_x_dir_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(77))];
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            get_x_dir(&args)
        });
        let value = result.expect("GetXDir handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn set_x_dir_stores_subpixel_fixed_velocity_like_cpp() {
        // C++ FnSetXDir(15) with default precision 10 sets xdir = itofix(15, 10)
        // = 1.5 px/frame (raw 16.16 value 98304). `C4Script.cpp:697`.
        let args = [Value::Int(15)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        assert_eq!(result.expect("SetXDir succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        // Component-only write (C4Script.cpp:697-705): ydir is untouched.
        let fixed_x = update.fixed_velocity_x.expect("fixed x recorded");
        assert_eq!(fixed_x, itofix_prec(15, 10));
        assert_eq!(fixed_x.val(), 98304);
        assert!(update.fixed_velocity.is_none());
        assert!(update.fixed_velocity_y.is_none());
        // The whole-pixel mirror derives at the fold (fixtoi of the
        // final fixed value) — no int velocity staged here.
        assert!(update.velocity.is_none());
    }

    #[test]
    fn set_y_dir_applies_precision_when_recording_update() {
        // C++ FnSetYDir(5, prec = 5) sets ydir = itofix(5, 5) = 1.0 px/frame
        // (raw 16.16 value 65536). `C4Script.cpp:723`.
        let args = [Value::Int(5), Value::Nil, Value::Int(5)];
        let (result, outcome) = with_object_host_context(|| set_y_dir(&args));
        let value = result.expect("SetYDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        // Component-only write (C4Script.cpp:718-732): xdir untouched, the
        // whole-pixel mirror derives at the fold from the final fixed value.
        let fixed_y = update.fixed_velocity_y.expect("fixed y recorded");
        assert_eq!(fixed_y, itofix_prec(5, 5));
        assert_eq!(fixed_y.val(), 65536);
        assert!(update.fixed_velocity.is_none());
        assert!(update.fixed_velocity_x.is_none());
    }

    #[test]
    fn set_r_dir_stores_subpixel_rotation_velocity_like_cpp() {
        // C++ FnSetRDir(10) with default precision 10 sets rdir = itofix(10, 10)
        // = 1.0 deg/frame (raw 16.16 value 65536). `C4Script.cpp:710`.
        let args = [Value::Int(10)];
        let (result, outcome) = with_object_host_context(|| set_r_dir(&args));
        assert_eq!(result.expect("SetRDir succeeds"), Value::Bool(true));
        let update = outcome
            .object_update
            .expect("rotation velocity update recorded");
        let rdir = update
            .rotation_velocity
            .expect("rotation velocity recorded");
        assert_eq!(rdir, itofix_prec(10, 10));
        assert_eq!(rdir.val(), 65536);
    }

    #[test]
    fn get_r_dir_reflects_pending_set_r_dir() {
        // Within a call, GetRDir reflects a prior SetRDir: SetRDir(10) is
        // 1.0 deg/frame, so GetRDir() at default precision 10 returns 10.
        let (result, _) = with_object_host_context(|| {
            set_r_dir(&[Value::Int(10)])?;
            get_r_dir(&[])
        });
        assert_eq!(result.expect("GetRDir succeeds"), Value::Int(10));
    }

    #[test]
    fn set_x_dir_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(4), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        let value = result.expect("SetXDir returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_r_dir_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(4), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_r_dir(&args));
        let value = result.expect("SetRDir returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_records_object_update() {
        let args = [Value::Int(15), Value::Int(27)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(15, 27)));
    }

    #[test]
    fn set_position_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(42));
        let args = [Value::Int(5), Value::Int(6), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_clamps_coordinates_when_requested() {
        let landscape = Landscape::flat(4, 6);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Nil,
            Value::Bool(true),
        ];
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[ObjectVertex::new(0, 0)],
                crate::FULL_CON,
            )),
            &[],
            world,
            1,
            || set_position(&args),
        );

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(3, 6)));
    }

    #[test]
    fn get_x_rejects_additional_arguments() {
        let (result, _) = with_object_host_context(|| get_x(&[Value::Nil, Value::Nil]));
        let error = result.expect_err("GetX rejects extra arguments");
        assert_eq!(error.to_string(), "GetX expects at most 1 argument: target");
    }

    #[test]
    fn get_effect_uses_context_view() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(1),
            ])
        });

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn get_effect_returns_command_target_metadata() {
        let state = empty_state();
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(7));
        let target = Value::Proplist(target_map);

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::String("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(4),
            ])
        });
        let value = result.expect("GetEffect command target succeeds");
        assert_eq!(value, Value::Int(7));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::String("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(5),
            ])
        });
        let value = result.expect("GetEffect command id succeeds");
        assert_eq!(value, Value::String("BARL".into()));
    }

    #[test]
    fn get_effect_query_zero_returns_effect_number_like_cpp() {
        // FnGetEffect query 0 returns pEffect->iNumber (C4Script.cpp:5481)
        // — the per-object monotonic handle (C4Effect.cpp:76-78), NOT a
        // list position. Removing an earlier effect must not renumber the
        // survivor: Control2Effect feeds this handle straight into
        // EffectCall (Clonk.c4d/Script.c:860-875).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("First".into()), state.clone()])?;
            add_effect(&[Value::String("XControl".into()), state.clone()])?;
            remove_effect(&[Value::String("First".into()), state.clone()])?;
            get_effect(&[Value::String("*Control*".into()), state.clone()])
        });
        let value = result.expect("GetEffect succeeds");
        assert_eq!(
            value,
            Value::Int(2),
            "the surviving effect keeps its allocated iNumber"
        );
    }

    #[test]
    fn get_effect_without_name_resolves_by_number_like_cpp() {
        // FnGetEffect with no/empty name treats iIndex as the effect
        // NUMBER (C4Script.cpp:5471-5476 -> C4Effect::Get(iNumber, true),
        // C4Effect.cpp:240-256). Control2Effect's
        // `GetEffect(0, this(), iEffect, 1)` relies on it.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("First".into()), state.clone()])?;
            add_effect(&[Value::String("XControl".into()), state.clone()])?;
            remove_effect(&[Value::String("First".into()), state.clone()])?;
            get_effect(&[
                Value::Int(0),
                state.clone(),
                Value::Int(2),
                Value::Int(1),
            ])
        });
        let value = result.expect("GetEffect succeeds");
        assert_eq!(
            value,
            Value::String("XControl".into()),
            "number 2 resolves the effect even at list position 0"
        );
    }

    #[test]
    fn get_effect_count_filters_by_name_and_priority() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        let value = result.expect("GetEffectCount succeeds");
        assert_eq!(value, Value::Int(3));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::String("Glow".into()), state.clone()])
        });
        let value = result.expect("GetEffectCount with name succeeds");
        assert_eq!(value, Value::Int(1));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone(), Value::Int(90)])
        });
        let value = result.expect("GetEffectCount with priority succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn wildcard_match_follows_cpp_swildcardmatchex() {
        // SWildcardMatchEx (C4Strings.cpp:531-562) via FnWildcardMatch
        // (C4Script.cpp:5606-5609): `*`/`?` with backtracking, int 0/1 result.
        // Clonk.c4d Script.c:587 gates riding controls on
        // `WildcardMatch(GetAction(), "Ride*")`.
        let m = |s: &str, w: &str| {
            wildcard_match(&[Value::String(s.into()), Value::String(w.into())])
                .expect("WildcardMatch succeeds")
        };
        assert_eq!(m("Walk", "Ride*"), Value::Int(0));
        assert_eq!(m("Ride", "Ride*"), Value::Int(1));
        assert_eq!(m("RideStill", "Ride*"), Value::Int(1));
        assert_eq!(m("IntJnRAimControl", "*Control*"), Value::Int(1));
        assert_eq!(m("abc", "*b"), Value::Int(0));
        assert_eq!(m("ab", "a?"), Value::Int(1));
        assert_eq!(m("ab", "*"), Value::Int(1));
        assert_eq!(m("", ""), Value::Int(1));
        // FnStringPar maps nil (and Set0'd falsy pars) to "" (C4Script.cpp:78-81).
        assert_eq!(
            wildcard_match(&[Value::Nil, Value::Int(0)]).expect("falsy args succeed"),
            Value::Int(1)
        );
        assert_eq!(
            wildcard_match(&[Value::String("x".into()), Value::Nil]).expect("nil wildcard"),
            Value::Int(0)
        );
    }

    #[test]
    fn effect_name_filters_wildcard_match_like_cpp() {
        // C4Effect::Get/GetCount wildcard-compare effect names
        // (C4Effect.cpp:229,263 via SWildcardMatchEx), and FnRemoveEffect
        // resolves named removals through the same Get (C4Script.cpp:5494);
        // CLNK Control2Effect relies on `GetEffect("*Control*", this(), i)`.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("IntJnRAimControl".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(50)])?;
            let count = get_effect_count(&[Value::String("*Control*".into()), state.clone()])?;
            assert_eq!(count, Value::Int(1));
            let number = get_effect(&[Value::String("*Control*".into()), state.clone()])?;
            assert!(matches!(number, Value::Int(n) if n > 0));
            remove_effect(&[Value::String("*Contr?l*".into()), state.clone()])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        assert_eq!(
            result.expect("wildcard filter chain succeeds"),
            Value::Int(1)
        );
    }

    #[test]
    fn get_effect_count_accepts_falsy_name_like_cpp_set0() {
        // Pre-#strict-3 scripts pass falsy values where a C4String* is
        // expected; C4AulExec.cpp:1370-1374 Set0()s them to nil before
        // conversion, so `GetEffectCount(0, this())` (Clonk.c4d Script.c:863
        // Control2Effect) counts all effects like a nil name.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            get_effect_count(&[Value::Int(0), state.clone()])
        });
        let value = result.expect("GetEffectCount with falsy int name succeeds");
        assert_eq!(value, Value::Int(2));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            get_effect_count(&[Value::Bool(false), state.clone()])
        });
        let value = result.expect("GetEffectCount with falsy bool name succeeds");
        assert_eq!(value, Value::Int(1));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            get_effect_count(&[Value::Int(7), state.clone()])
        });
        result.expect_err("GetEffectCount with truthy int name errors like C++ ConvertTo");
    }

    #[test]
    fn get_and_remove_effect_accept_falsy_name_like_cpp_set0() {
        // Same Set0 path as above: `GetEffect(0, this(), i)` follows the
        // GetEffectCount(0, …) call in Control2Effect (Clonk.c4d Script.c:868)
        // and JumpAndRun.c:86 calls `RemoveEffect(0, this(), number)`. A
        // falsy name means BY-NUMBER resolution (C4Script.cpp:5474-5476,
        // 5502-5507): the AddEffect handle round-trips, number 0 finds
        // nothing (numbers start at 1, C4Effect.cpp:76-78).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let number =
                add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            assert!(matches!(number, Value::Int(n) if n > 0));
            assert_eq!(
                get_effect(&[Value::Int(0), state.clone(), number.clone()])?,
                number,
                "the AddEffect handle resolves by number"
            );
            assert_eq!(
                get_effect(&[Value::Int(0), state.clone()])?,
                Value::Nil,
                "number 0 matches no effect like C4Effect::Get"
            );
            remove_effect(&[Value::Int(0), state.clone(), number])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        let value = result.expect("falsy-name GetEffect/RemoveEffect chain succeeds");
        assert_eq!(value, Value::Int(0));
    }

    #[test]
    fn get_effect_count_reads_state_snapshot_when_no_context() {
        let mut glow = HashMap::new();
        glow.insert("name".into(), Value::String("Glow".into()));
        glow.insert("priority".into(), Value::Int(100));
        glow.insert("interval".into(), Value::Int(1));
        glow.insert("timer".into(), Value::Int(0));

        let mut spark = HashMap::new();
        spark.insert("name".into(), Value::String("Spark".into()));
        spark.insert("priority".into(), Value::Int(60));
        spark.insert("interval".into(), Value::Int(1));
        spark.insert("timer".into(), Value::Int(0));

        let state = {
            let mut map = HashMap::new();
            map.insert(
                "effects".into(),
                Value::Array(vec![Value::Proplist(glow), Value::Proplist(spark)]),
            );
            Value::Proplist(map)
        };

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Nil])
            .expect("GetEffectCount without context succeeds");
        assert_eq!(value, Value::Int(2));

        let value = get_effect_count(&[Value::String("Spark".into()), state, Value::Int(50)])
            .expect("GetEffectCount with state filter succeeds");
        assert_eq!(value, Value::Int(0));
    }

    #[test]
    fn effect_var_reads_and_writes_values() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Spark".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(3),
                object_reference_value(ObjectId::new(44)),
            ])?;

            let initial = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])?;
            assert_eq!(initial, Value::Int(3));

            let object = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(object, object_reference_value(ObjectId::new(44)));

            let unset = effect_var(&[Value::Int(2), state.clone(), Value::Int(1)])?;
            assert_eq!(unset, Value::Nil);

            let updated = effect_var(&[
                Value::Int(2),
                state.clone(),
                Value::Int(1),
                Value::String("beam".into()),
            ])?;
            assert_eq!(updated, Value::String("beam".into()));

            let reread = effect_var(&[Value::Int(2), state.clone(), Value::Int(1)])?;
            assert_eq!(reread, Value::String("beam".into()));

            Ok(Value::Nil)
        });

        result.expect("EffectVar interactions succeed");
        assert_eq!(outcome.object.len(), 2);
        match &outcome.object[1] {
            // EffectVar writes fold as number-keyed UPDATEs — an Add
            // would resurrect an effect killed earlier the same frame.
            EffectCommand::Update(effect) => {
                assert_eq!(effect.vars().len(), 3);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(3));
                assert_eq!(effect.vars()[1], EffectVarValue::Object(44));
                assert_eq!(effect.vars()[2], EffectVarValue::String("beam".into()));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn effect_var_reads_from_state_without_context() {
        let mut effect_map = HashMap::new();
        effect_map.insert("name".into(), Value::String("Glow".into()));
        effect_map.insert("priority".into(), Value::Int(80));
        effect_map.insert("interval".into(), Value::Int(1));
        effect_map.insert("timer".into(), Value::Int(0));
        effect_map.insert(
            "vars".into(),
            Value::Array(vec![Value::Int(9), Value::String("pulse".into())]),
        );

        let mut state_map = HashMap::new();
        state_map.insert(
            "effects".into(),
            Value::Array(vec![Value::Proplist(effect_map.clone())]),
        );
        let state = Value::Proplist(state_map);

        let read_value = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])
            .expect("EffectVar read succeeds");
        assert_eq!(read_value, Value::Int(9));

        let read_string = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])
            .expect("EffectVar string read succeeds");
        assert_eq!(read_string, Value::String("pulse".into()));

        let set_result = effect_var(&[Value::Int(0), state, Value::Int(1), Value::Int(5)]);
        assert!(set_result.is_err());
    }

    #[test]
    fn set_action_records_object_update() {
        let args = vec![Value::String("Walk".into())];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction should succeed");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("action update present");
        let action = update.action.expect("action delta present");
        assert_eq!(action.name.as_deref(), Some("Walk"));
        assert!(outcome.object_commands.is_empty());
        assert!(!outcome.destroy_object);
    }

    #[test]
    fn set_action_sets_the_action_targets_like_cpp() {
        // FnSetAction (C4Script.cpp:747-753): the object arguments are the
        // ACTION's targets — SetActionByName(name, pTarget, pTarget2) —
        // never a which-object guard.
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(2));
        let args = vec![Value::String("Jump".into()), Value::Proplist(target_map)];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(2))));
    }

    #[test]
    fn set_action_nil_targets_preserve_the_existing_ones_like_cpp() {
        // C4Object::SetAction assigns the targets ONLY when given
        // (C4Object.cpp:4123-4125: `if (pTarget) Action.Target = pTarget;`)
        // — SetAction(name, nil, nil) keeps the previous targets, for
        // explicit nils and omitted arguments alike.
        let args = vec![Value::String("Jump".into()), Value::Nil, Value::Nil];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, None, "nil target1 must not stage a clear");
        assert_eq!(action.target2, None, "nil target2 must not stage a clear");
    }

    #[test]
    fn set_object_status_records_update() {
        let args = vec![Value::Int(ObjectStatus::Inactive.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.expect("SetObjectStatus succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("status update present");
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn set_object_status_rejects_deleted() {
        let args = vec![Value::Int(ObjectStatus::Deleted.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.expect("SetObjectStatus returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_object_status_reflects_pending_update() {
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let set_value =
                set_object_status(&[Value::Int(ObjectStatus::Inactive.to_script_value())])?;
            assert_eq!(set_value, Value::Bool(true));
            get_object_status(&[])
        });

        let value = result.expect("GetObjectStatus succeeds");
        assert_eq!(value, Value::Int(ObjectStatus::Inactive.to_script_value()));
        let update = outcome.object_update.expect("status update present");
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn get_owner_returns_current_owner() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                5,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_owner(&[]),
        );

        let value = result.expect("GetOwner succeeds");
        assert_eq!(value, Value::Int(5));
    }

    #[test]
    fn get_owner_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            42,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_owner(&args));

        let value = result.expect("GetOwner for target succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn set_owner_records_owner_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                1,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_owner(&[Value::Int(3)]),
        );

        let value = result.expect("SetOwner returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("owner update recorded");
        assert_eq!(update.owner, Some(3));
    }

    #[test]
    fn set_owner_respects_target_filter() {
        let world = HostWorldContext::default();
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(2), Value::Proplist(target)];

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            world,
            1,
            || set_owner(&args),
        );

        let value = result.expect("SetOwner returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_alive_records_alive_update() {
        let (result, outcome) = with_effect_context(
            Some(
                HostObjectContext::new(
                    ObjectId::new(1),
                    None,
                    ObjectStatus::Normal,
                    100,
                    OWNER_NONE,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    &[],
                    "Idle",
                    0,
                    0,
                    ActionLibrary::default(),
                    Direction::Left,
                    CommandDirection::Stop,
                    0,
                    None,
                    None,
                    &[],
                    crate::FULL_CON,
                )
                .with_alive(true),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || set_alive(&[Value::Bool(false)]),
        );

        let value = result.expect("SetAlive returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("alive update recorded");
        assert_eq!(update.alive, Some(false));
    }

    #[test]
    fn set_alive_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(42));
        let args = [Value::Bool(true), Value::Proplist(target)];

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_alive(&args),
        );

        let value = result.expect("SetAlive returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_alive_returns_current_state() {
        let (result, _) = with_effect_context(
            Some(
                HostObjectContext::new(
                    ObjectId::new(1),
                    None,
                    ObjectStatus::Normal,
                    100,
                    OWNER_NONE,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    &[],
                    "Idle",
                    0,
                    0,
                    ActionLibrary::default(),
                    Direction::Left,
                    CommandDirection::Stop,
                    0,
                    None,
                    None,
                    &[],
                    crate::FULL_CON,
                )
                .with_alive(false),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || get_alive(&[]),
        );

        let value = result.expect("GetAlive returns bool");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn get_alive_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_alive(false)]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_alive(&args));

        let value = result.expect("GetAlive for target succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn physicals_host_fns_follow_cpp_mode_semantics() {
        // SetPhysical/GetPhysical/TrainPhysical/ResetPhysical
        // (C4Script.cpp:552-688) on a non-crew object with all-zero
        // definition physicals.
        let (result, outcome) = with_object_host_context(|| {
            let walk = || Value::String("Walk".to_string());
            // TrainPhysical with neither temp mode nor info trains nothing
            // (C4Object.cpp:2136-2146).
            assert_eq!(
                train_physical(&[walk(), Value::Int(5), Value::Int(C4_MAX_PHYSICAL)])?,
                Value::Bool(false)
            );
            // Unknown physical name fails (C4Script.cpp:562).
            assert_eq!(
                set_physical(&[
                    Value::String("Bogus".to_string()),
                    Value::Int(1),
                    Value::Int(2)
                ])?,
                Value::Bool(false)
            );
            // PHYS_Current needs temp mode or an info (C4Script.cpp:569).
            assert_eq!(
                set_physical(&[walk(), Value::Int(1), Value::Int(0)])?,
                Value::Bool(false)
            );
            // PHYS_Permanent needs an info (C4Script.cpp:576).
            assert_eq!(
                set_physical(&[walk(), Value::Int(1), Value::Int(1)])?,
                Value::Bool(false)
            );
            // PHYS_Temporary reads need an info too (C4Script.cpp:680).
            assert_eq!(get_physical(&[walk(), Value::Int(2)])?, Value::Nil);
            // PHYS_Temporary write auto-enables temp mode
            // (C4Script.cpp:587-596).
            assert_eq!(
                set_physical(&[walk(), Value::Int(50_000), Value::Int(2)])?,
                Value::Bool(true)
            );
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(50_000));
            // PHYS_Current works while temp mode is on (C4Script.cpp:567-572).
            assert_eq!(
                set_physical(&[walk(), Value::Int(60_000), Value::Int(0)])?,
                Value::Bool(true)
            );
            // PHYS_StackTemporary registers the previous value
            // (C4Script.cpp:593-596).
            assert_eq!(
                set_physical(&[walk(), Value::Int(70_000), Value::Int(3)])?,
                Value::Bool(true)
            );
            // Training in temp mode trains the active value AND the stacked
            // previous one (C4InfoCore.cpp:309-317).
            assert_eq!(
                train_physical(&[walk(), Value::Int(5), Value::Int(C4_MAX_PHYSICAL)])?,
                Value::Bool(true)
            );
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(70_005));
            // Named reset restores the last stacked value
            // (C4Script.cpp:622-629; C4InfoCore.cpp:339-351) and keeps temp
            // mode because the set still deviates from the reference.
            assert_eq!(reset_physical(&[Value::Nil, walk()])?, Value::Bool(true));
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(60_005));
            // Full reset drops temp mode (C4Script.cpp:631-635)...
            assert_eq!(reset_physical(&[])?, Value::Bool(true));
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(0));
            // ...and resetting without temp mode fails (C4Script.cpp:619).
            assert_eq!(reset_physical(&[])?, Value::Bool(false));
            Ok(Value::Nil)
        });
        result.expect("physicals host fns run");
        // The scope records the final physical state for the engine — the
        // cleared temp mode must overwrite any prior engine-side state.
        let update = outcome.object_update.expect("physicals update recorded");
        let physicals = update.physicals.expect("physicals state recorded");
        assert_eq!(physicals.info, None);
        assert_eq!(physicals.temporary, None);
        assert_eq!(physicals.changes, Vec::<(String, i32)>::new());
    }

    #[test]
    fn do_energy_applies_delta_and_clamps() {
        // The harness object carries 100 raw energy and NO physical:
        // DoEnergy(-25) = -25% = -25000 raw (C4Object.cpp:1347), floored
        // at 0; the upper clamp needs GetPhysical()->Energy (the
        // physical-less fixture keeps raw growth unclamped, documented).
        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(-25)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(0));

        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(50)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(50_100));
    }

    #[test]
    fn do_energy_respects_target_argument() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(-10), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_energy(&args));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_damage_applies_delta_and_clamps() {
        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(15)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(15));

        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(-20)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(0));
    }

    #[test]
    fn do_damage_respects_target_argument() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(77));
        let args = [Value::Int(5), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_damage(&args));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_energy_accepts_exact_flag() {
        let args = [Value::Int(0), Value::Nil, Value::Bool(true)];
        let (result, outcome) = with_object_host_context(|| do_energy(&args));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        assert!(outcome
            .object_update
            .as_ref()
            .and_then(|update| update.energy)
            .is_some());
    }

    #[test]
    fn get_energy_returns_current_energy() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                // 75% of C4MaxPhysical - GetEnergy reads percent
                // (C4Script.cpp FnGetEnergy).
                75_000,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy succeeds");
        assert_eq!(value, Value::Int(75));
    }

    #[test]
    fn get_con_returns_current_construction() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON / 2,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_con(&[]),
        );

        let value = result.expect("GetCon succeeds");
        assert_eq!(value, Value::Int(50));
    }

    #[test]
    fn do_con_adjusts_construction() {
        let (result, outcome) = with_object_host_context(|| do_con(&[Value::Int(-25)]));
        let value = result.expect("DoCon returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome
            .object_update
            .expect("DoCon should produce an object update");
        let expected = crate::FULL_CON - ((crate::FULL_CON * 25) / 100);
        assert_eq!(update.construction, Some(expected));
    }

    #[test]
    fn get_energy_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(55),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            33_000,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [object_reference_value(ObjectId::new(55))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_energy(&args));

        let value = result.expect("GetEnergy target succeeds");
        assert_eq!(value, Value::Int(33));
    }

    #[test]
    fn get_energy_returns_nil_without_context() {
        let (result, _) =
            with_effect_context(
                None,
                &[],
                HostWorldContext::default(),
                1,
                || get_energy(&[]),
            );
        let value = result.expect("GetEnergy handles missing context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_energy_converts_raw_units_to_percent() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(3),
                None,
                ObjectStatus::Normal,
                LEGACY_MAX_PHYSICAL / 2,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy converts raw energy");
        assert_eq!(value, Value::Int(50));
    }

    #[test]
    fn create_object_registers_spawn_and_returns_reference() {
        let args = [Value::String("Clonk".into())];
        let (result, outcome) = with_object_host_context(|| create_object(&args));
        let value = result.expect("CreateObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "Clonk");
        assert_eq!(spawn.position, Vector2::ZERO);
        // The defaulted C4ValueInt owner is 0 (FnCreateObject substitutes
        // the caller's owner only for NONSTRICT scripts).
        assert_eq!(spawn.owner, 0);
        assert_eq!(spawn.id, Some(ObjectId::new(1)));
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_construction_registers_spawn_when_site_valid() {
        let landscape = Landscape::flat(64, 50);
        let definitions = HashMap::from([(
            "Workshop".to_string(),
            DefinitionMetadata {
                name: String::new(),
                category: crate::CATEGORY_STRUCTURE,
                ocf_base: ocf::NORMAL,
                crew_member: false,
                action_library: ActionLibrary::default(),
                value: 0,
                mass: 100,
                constructable: true,
                shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
                construction_offset: 0,
                basement: 0,
                physical: PhysicalInfo::default(),
                components: Vec::new(),
                line_connect: 0,
                clonk_name_newlines: None,
                stretch_growth: false,
                line: 0,
                vertices: Vec::new(),
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::String("Workshop".into()),
            Value::Int(32),
            Value::Int(50),
            Value::Int(1),
            Value::Int(0),
            Value::Bool(false),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_construction(&args));
        let value = result.expect("CreateConstruction succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "Workshop");
        assert_eq!(spawn.position, Vector2::new(32, 50));
        assert_eq!(spawn.owner, 1);
        assert_eq!(spawn.construction, 0);
        assert_eq!(spawn.category, Some(crate::CATEGORY_STRUCTURE));
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_construction_returns_nil_when_site_blocked() {
        let landscape = Landscape::flat(64, 50);
        let workshop_metadata = DefinitionMetadata {
            name: String::new(),
            category: crate::CATEGORY_STRUCTURE,
            ocf_base: ocf::NORMAL,
            crew_member: false,
            action_library: ActionLibrary::default(),
            value: 0,
            mass: 100,
            constructable: true,
            shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
            construction_offset: 0,
            basement: 0,
            physical: PhysicalInfo::default(),
            components: Vec::new(),
            line_connect: 0,
            clonk_name_newlines: None,
            stretch_growth: false,
            line: 0,
            vertices: Vec::new(),
        };
        let definitions = HashMap::from([
            ("Workshop".to_string(), workshop_metadata.clone()),
            ("Existing".to_string(), workshop_metadata),
        ]);
        let existing = HostWorldObject::with_category(
            ObjectId::new(10),
            "Existing",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            crate::CATEGORY_STRUCTURE,
            0,
            crate::FULL_CON,
            0,
            Vector2::new(32, 50),
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0, // action_phase
            None,
            None,
        );
        let world = HostWorldContext::with_landscape(
            vec![existing],
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::String("Workshop".into()),
            Value::Int(32),
            Value::Int(50),
            Value::Int(1),
            Value::Int(0),
            Value::Bool(false),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_construction(&args));
        let value = result.expect("CreateConstruction completes");
        assert_eq!(value, Value::Nil);
        assert!(outcome.spawns.is_empty());
        assert_eq!(outcome.next_object_id, 1);
    }

    #[test]
    fn create_particle_registers_command() {
        let args = [
            Value::String("Smoke".into()),
            Value::Int(8),
            Value::Int(-4),
            Value::Int(20),
            Value::Int(-10),
            Value::Int(15),
            Value::Int(60),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert_eq!(config.definition_id, "Smoke");
                assert_eq!(config.position, FloatVector2::new(8.0, -4.0));
                assert_eq!(config.velocity, FloatVector2::new(2.0, -1.0));
                assert_eq!(config.parameter_a, 1.5);
                assert_eq!(config.parameter_b, 60);
                assert_eq!(config.life, 60);
                assert!(matches!(config.layer, ParticleLayer::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_with_object_sets_layer() {
        let target_id = ObjectId::new(5);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Torch",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(30),
            object_reference_value(target_id),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert!(matches!(
                    config.layer,
                    ParticleLayer::ObjectBack(id) if id == target_id
                ));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_rejects_unknown_object() {
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(10),
            object_reference_value(ObjectId::new(99)),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle handles missing object");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.particles.is_empty());
    }

    fn find_world_object(id: u64, definition: &str, x: i32, y: i32, owner: i32) -> HostWorldObject {
        HostWorldObject::new(
            ObjectId::new(id),
            definition,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            owner,
            100,
            crate::FULL_CON,
            Vector2::new(x, y),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
    }

    #[test]
    fn criterion_parsing_stops_at_first_nil_par_like_cpp() {
        // CreateCriterionsFromPars stops at the first nil parameter
        // (`if (!Data) break;`, C4Script.cpp:1996): criteria after a nil
        // argument are never parsed.
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 2),
            find_world_object(3, "ROCK", 90, 10, 2),
        ]);
        // [ID ROCK], nil, [Owner 2]: C++ uses only the ROCK criterion, so
        // the first rock (object 1) wins — not the owner-2 rock (object 3).
        let args = vec![
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
            Value::Nil,
            Value::Array(vec![Value::Int(50), Value::Int(2)]),
        ];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(1))
        );
    }

    #[test]
    fn find_object2_condition_tree_matches_cpp() {
        // C4FindObject::CreateByValue (C4FindObject.cpp:37-162) +
        // CreateCriterionsFromPars (C4Script.cpp:1985-2034): criteria arrays
        // AND together; Not/Or nest; the first main-list match wins
        // (C4FindObject.cpp:188-194).
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 2),
            find_world_object(3, "ROCK", 90, 10, 2),
        ]);
        // [C4FO_ID(20), "ROCK"] AND [C4FO_Owner(50), 2] → object 3
        let args = vec![
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
            Value::Array(vec![Value::Int(50), Value::Int(2)]),
        ];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(3))
        );

        // [C4FO_Not(1), [C4FO_ID, "ROCK"]] → first non-rock (object 2)
        let args = vec![Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(2))
        );

        // [C4FO_Or(3), [ID TREE], [InRect around object 3]] → objects 2 and 3
        let args = vec![Value::Array(vec![
            Value::Int(3),
            Value::Array(vec![Value::Int(20), Value::String("TREE".into())]),
            Value::Array(vec![
                Value::Int(10),
                Value::Int(85),
                Value::Int(5),
                Value::Int(10),
                Value::Int(10),
            ]),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || object_count2(&args));
        assert_eq!(result.expect("ObjectCount2 succeeds"), Value::Int(2));

        // No valid criterions → script error (C4Script.cpp:2042-2043)
        let (result, _) =
            with_object_host_context_with_world(world, || find_object2(&[Value::Int(5)]));
        assert!(result.is_err());
    }

    #[test]
    fn find_objects2_sort_random_consumes_synced_draws_in_collection_order() {
        // C4SortObjectRandom::CompareGetValue draws the synced
        // Random(1 << 16) (C4FindObject.cpp:914-917) — once per object via
        // the PrepareCache pass in collection order
        // (C4FindObject.cpp:819-832), then a stable ascending sort.
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "ROCK", 50, 10, 1),
            find_world_object(3, "ROCK", 90, 10, 1),
        ]);
        let args = vec![
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
            Value::Array(vec![Value::Int(120)]), // C4SO_Random
        ];
        let rng = LcgRng::seed_from_u64(99);
        let mut mirror = rng.clone();
        let guard = enter_random_context(rng);
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let rng_after = guard.finish();
        let draws = [
            mirror.random(1 << 16),
            mirror.random(1 << 16),
            mirror.random(1 << 16),
        ];
        assert_eq!(rng_after, mirror, "exactly one draw per object, in order");
        // ascending by drawn value, stable
        let mut expected: Vec<(i32, u64)> = draws
            .iter()
            .zip([1u64, 2, 3])
            .map(|(&draw, id)| (draw, id))
            .collect();
        expected.sort_by_key(|&(draw, _)| draw);
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        let ids: Vec<Option<ObjectId>> = values.iter().map(object_id_from_value).collect();
        let expected_ids: Vec<Option<ObjectId>> = expected
            .iter()
            .map(|&(_, id)| Some(ObjectId::new(id)))
            .collect();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn find_objects2_sort_mass_and_reverse_match_cpp() {
        // C4SO_Mass sorts lightest first (C4FindObject.h:59, ascending by
        // CompareGetValue); C4SO_Reverse flips it (C4FindObject.cpp:856-869).
        let definitions: HashMap<DefinitionId, DefinitionMetadata> = [
            (
                "LGHT".to_string(),
                DefinitionMetadata {
                    mass: 10,
                    ..DefinitionMetadata::default()
                },
            ),
            (
                "HEVY".to_string(),
                DefinitionMetadata {
                    mass: 500,
                    ..DefinitionMetadata::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        let world = HostWorldContext::with_landscape(
            vec![
                find_world_object(1, "HEVY", 10, 10, 1),
                find_world_object(2, "LGHT", 50, 10, 1),
            ],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            10,
            false,
        );
        let all = Value::Array(vec![Value::Int(22), Value::Int(0xFFFF)]); // C4FO_Category any
        let args = vec![all.clone(), Value::Array(vec![Value::Int(140)])]; // C4SO_Mass
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("array result");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(2)), Some(ObjectId::new(1))],
            "lightest first"
        );

        // [C4SO_Reverse(101), [C4SO_Mass]] → heaviest first
        let args = vec![
            all,
            Value::Array(vec![Value::Int(101), Value::Array(vec![Value::Int(140)])]),
        ];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("array result");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(1)), Some(ObjectId::new(2))],
            "reverse: heaviest first"
        );
    }

    #[test]
    fn cast_particles_registers_cast_command_and_checks_def_registry() {
        // FnCastParticles (C4Script.cpp:4881-4903): args are
        // (name, amount, level, x, y, a0, a1, b0, b1, obj); a-values are
        // script ints /10; GetDef failure → false.
        let defs: std::collections::HashSet<String> = ["Mist".to_string()].into_iter().collect();
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs.clone());
        let args = [
            Value::String("Mist".into()),
            Value::Int(12),
            Value::Int(20),
            Value::Int(5),
            Value::Int(6),
            Value::Int(10),
            Value::Int(20),
            Value::Int(0x11223344),
            Value::Int(0x55667788),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Cast {
                definition_id,
                amount,
                x,
                y,
                level,
                a0,
                b0,
                a1,
                b1,
                layer,
            } => {
                assert_eq!(definition_id, "Mist");
                assert_eq!(*amount, 12);
                assert_eq!(*level, 20);
                assert_eq!(x.to_bits(), 5.0f32.to_bits());
                assert_eq!(y.to_bits(), 6.0f32.to_bits());
                assert_eq!(a0.to_bits(), 1.0f32.to_bits());
                assert_eq!(a1.to_bits(), 2.0f32.to_bits());
                assert_eq!(*b0, 0x11223344);
                assert_eq!(*b1, 0x55667788);
                assert!(matches!(layer, ParticleLayer::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }

        // Unknown def with a registry attached → false, no command
        // (C4Script.cpp:4893).
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs);
        let args = [
            Value::String("NoSuchDef".into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(false));
        assert!(outcome.particles.is_empty());

        // No registry attached (legacy fixture context) → permissive.
        let args = [
            Value::String("Anything".into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, outcome) = with_object_host_context(|| cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
    }

    #[test]
    fn cast_back_particles_targets_back_layer() {
        // FnCastBackParticles (C4Script.cpp:4905-4908) = FnCastAParticles
        // with fBack = true → the object's BackParticles list.
        let target_id = ObjectId::new(9);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Engine",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Exhaust".into()),
            Value::Int(3),
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            object_reference_value(target_id),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_back_particles(&args));
        assert_eq!(
            result.expect("CastBackParticles succeeds"),
            Value::Bool(true)
        );
        match &outcome.particles[0] {
            ParticleCommand::Cast { layer, .. } => {
                assert!(matches!(layer, ParticleLayer::ObjectBack(id) if *id == target_id));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn push_particles_registers_push_command_and_checks_def_registry() {
        // FnPushParticles (C4Script.cpp:4910-4923): nil name pushes all
        // particles; deltas are script ints /10; a named def that is not
        // loaded → false.
        let (result, outcome) = with_object_host_context(|| {
            push_particles(&[Value::Nil, Value::Int(15), Value::Int(-5)])
        });
        assert_eq!(result.expect("PushParticles succeeds"), Value::Bool(true));
        match &outcome.particles[0] {
            ParticleCommand::Push {
                definition_id,
                dxdir,
                dydir,
            } => {
                assert!(definition_id.is_none());
                assert_eq!(dxdir.to_bits(), 1.5f32.to_bits());
                assert_eq!(dydir.to_bits(), (-0.5f32).to_bits());
            }
            other => panic!("unexpected particle command {other:?}"),
        }

        let defs: std::collections::HashSet<String> = ["Spark".to_string()].into_iter().collect();
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            push_particles(&[
                Value::String("Missing".into()),
                Value::Int(0),
                Value::Int(0),
            ])
        });
        assert_eq!(result.expect("PushParticles succeeds"), Value::Bool(false));
        assert!(outcome.particles.is_empty());
    }

    #[test]
    fn clear_particles_registers_command() {
        let (result, outcome) = with_object_host_context(|| clear_particles(&[]));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert!(definition_id.is_none());
                assert!(matches!(scope, ParticleScope::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn clear_particles_with_object_sets_scope() {
        let target_id = ObjectId::new(12);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Emitter",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Smoke".into()),
            object_reference_value(target_id),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || clear_particles(&args));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert_eq!(definition_id.as_deref(), Some("Smoke"));
                assert!(matches!(scope, ParticleScope::Object(id) if *id == target_id));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn contained_returns_nil_when_object_has_no_container() {
        let (result, _) = with_object_host_context(|| contained(&[]));
        let value = result.expect("Contained without container succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn contained_returns_container_reference() {
        let container_id = ObjectId::new(42);
        let object_id = ObjectId::new(7);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container_id,
                "Chest",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                object_id,
                "Gem",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container_id),
            ),
        ]);
        let context = HostObjectContext::new(
            object_id,
            Some(container_id),
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let (result, _) = with_effect_context(Some(context), &[], world, 100, || contained(&[]));
        let value = result.expect("Contained with container succeeds");
        assert_eq!(value, object_reference_value(container_id));
    }

    #[test]
    fn contents_skips_attached_by_default() {
        let container_id = ObjectId::new(100);
        let attached_id = ObjectId::new(101);
        let item_id = ObjectId::new(102);

        let container = HostWorldObject::new(
            container_id,
            "Crew",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![attached_id, item_id]);

        let attached = HostWorldObject::new(
            attached_id,
            "Banner",
            ObjectStatus::Normal,
            "Attach",
            None,
            None,
            Some("Attach".into()),
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let item = HostWorldObject::new(
            item_id,
            "Gem",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, attached, item]);
        let context = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Walk",
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );

        let (result, _) = with_effect_context(Some(context), &[], world, 200, || contents(&[]));
        let value = result.expect("Contents succeeds");
        assert_eq!(value, object_reference_value(item_id));
    }

    #[test]
    fn contents_includes_attached_when_requested() {
        let container_id = ObjectId::new(110);
        let attached_id = ObjectId::new(111);

        let container = HostWorldObject::new(
            container_id,
            "Crew",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![attached_id]);

        let attached = HostWorldObject::new(
            attached_id,
            "Banner",
            ObjectStatus::Normal,
            "Attach",
            None,
            None,
            Some("Attach".into()),
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, attached]);
        let context = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Walk",
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );

        let args = [Value::Nil, Value::Nil, Value::Bool(true)];
        let (result, _) = with_effect_context(Some(context), &[], world, 200, || contents(&args));
        let value = result.expect("Contents with attachments succeeds");
        assert_eq!(value, object_reference_value(attached_id));
    }

    #[test]
    fn contents_count_filters_by_definition() {
        let container_id = ObjectId::new(120);
        let gem_id = ObjectId::new(121);
        let hammer_id = ObjectId::new(122);

        let container = HostWorldObject::new(
            container_id,
            "Chest",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![gem_id, hammer_id]);

        let gem = HostWorldObject::new(
            gem_id,
            "Gem",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let hammer = HostWorldObject::new(
            hammer_id,
            "Hammer",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, gem, hammer]);
        let context_all = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );
        let context_filtered = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );

        let (result, _) = with_effect_context(Some(context_all), &[], world.clone(), 300, || {
            contents_count(&[])
        });
        let value = result.expect("ContentsCount without filter succeeds");
        assert_eq!(value, Value::Int(2));

        let args = [Value::String("Gem".into())];
        let (filtered, _) = with_effect_context(Some(context_filtered), &[], world, 300, || {
            contents_count(&args)
        });
        let filtered_value = filtered.expect("ContentsCount with filter succeeds");
        assert_eq!(filtered_value, Value::Int(1));
    }

    #[test]
    fn find_contents_returns_matching_object() {
        let container_id = ObjectId::new(130);
        let gem_id = ObjectId::new(131);
        let hammer_id = ObjectId::new(132);

        let container = HostWorldObject::new(
            container_id,
            "Chest",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![hammer_id, gem_id]);

        let hammer = HostWorldObject::new(
            hammer_id,
            "Hammer",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let gem = HostWorldObject::new(
            gem_id,
            "Gem",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, hammer, gem]);
        let context = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );

        let args = [Value::String("Gem".into())];
        let (result, _) =
            with_effect_context(Some(context), &[], world, 400, || find_contents(&args));
        let value = result.expect("FindContents succeeds");
        assert_eq!(value, object_reference_value(gem_id));
    }

    #[test]
    fn find_other_contents_returns_first_non_matching_object() {
        let container_id = ObjectId::new(140);
        let gem_id = ObjectId::new(141);
        let hammer_id = ObjectId::new(142);

        let container = HostWorldObject::new(
            container_id,
            "Chest",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![gem_id, hammer_id]);

        let gem = HostWorldObject::new(
            gem_id,
            "Gem",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let hammer = HostWorldObject::new(
            hammer_id,
            "Hammer",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, gem, hammer]);
        let context = HostObjectContext::new(
            container_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            crate::FULL_CON,
        );

        let args = [Value::String("Gem".into())];
        let (result, _) = with_effect_context(Some(context), &[], world, 500, || {
            find_other_contents(&args)
        });
        let value = result.expect("FindOtherContents succeeds");
        assert_eq!(value, object_reference_value(hammer_id));
    }

    #[test]
    fn remove_object_marks_destroy_flag() {
        let (result, outcome) = with_object_host_context(|| remove_object(&[]));
        assert_eq!(result.expect("RemoveObject succeeds"), Value::Bool(true));
        assert!(outcome.destroy_object);
    }

    #[test]
    fn find_object_returns_first_matching_definition() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(1),
                "Flag",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(10, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(2),
                "Rock",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(50, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);

        let args = [Value::String("Flag".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
    }

    #[test]
    fn find_object_has_no_owner_parameter_like_cpp() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(10),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(11),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                2,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        // FnFindObject has NO owner parameter — C++ always searches with
        // ANY_OWNER (C4Script.cpp:2133); only FindObjectOwner filters.
        let args = [
            Value::String("Dummy".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(2),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(
            value,
            object_reference_value(ObjectId::new(10)),
            "the trailing int is beyond pFindNext and ignored; owner never filters"
        );
    }

    #[test]
    fn find_object_closest_mode_orders_by_distance() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(20),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(2, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(21),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(6, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::String("Dummy".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
        ];
        let (first_result, _) =
            with_effect_context(None, &[], world.clone(), 1, || find_object(&args));
        let first_value = first_result.expect("FindObject closest succeeds");
        assert_eq!(first_value, object_reference_value(ObjectId::new(20)));

        let mut find_next = HashMap::new();
        find_next.insert("id".into(), Value::Int(20));
        let args_with_next = [
            Value::String("Dummy".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            // pFindNext is FindObject's 10th parameter (C4Script.cpp:2113).
            Value::Proplist(find_next),
        ];
        let (second_result, _) =
            with_effect_context(None, &[], world, 1, || find_object(&args_with_next));
        let second_value = second_result.expect("FindObject closest with next succeeds");
        assert_eq!(second_value, object_reference_value(ObjectId::new(21)));
    }

    #[test]
    fn find_object_respects_ocf_filter() {
        let matching_id = ObjectId::new(51);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                matching_id,
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )
            .with_ocf(ocf::AVAILABLE | ocf::ALIVE),
            HostWorldObject::new(
                ObjectId::new(52),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(ocf::AVAILABLE as i32),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(value, object_reference_value(matching_id));
    }

    #[test]
    fn find_object_point_uses_sector_shape_candidates() {
        let id = ObjectId::new(61);
        let mut definitions = HashMap::new();
        definitions.insert(
            "Wide".to_string(),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-10, -5, 20, 10)),
                ..DefinitionMetadata::default()
            },
        );
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                id,
                "Wide",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(40, 10),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            Some(Landscape::flat(120, 120)),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );

        let args = [
            Value::String("Wide".into()),
            Value::Int(31),
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        assert_eq!(
            result.expect("FindObject succeeds"),
            object_reference_value(id)
        );
    }

    #[test]
    fn find_objects_sector_range_uses_cpp_sector_enumeration_order() {
        // C4FindObject::FindMany with bounds and no sort pushes results in
        // AREA-ENUMERATION order — sector by sector (C4FindObject.cpp:
        // 344-353 via C4LArea::Next, C4Sector.cpp:264-277), NOT master-list
        // order. `first` ranks earlier but sits in sector 1 (x=70); `second`
        // sits in sector 0 (x=10) and is therefore encountered first.
        let first = ObjectId::new(71);
        let second = ObjectId::new(72);
        let world = HostWorldContext::with_landscape(
            vec![
                HostWorldObject::new(
                    first,
                    "Dummy",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    OWNER_NONE,
                    100,
                    crate::FULL_CON,
                    Vector2::new(70, 10),
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                ),
                HostWorldObject::new(
                    second,
                    "Dummy",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    OWNER_NONE,
                    100,
                    crate::FULL_CON,
                    Vector2::new(10, 10),
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                ),
            ],
            Some(Landscape::flat(120, 120)),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::String("Dummy".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(120),
            Value::Int(20),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_objects(&args));
        let value = result.expect("FindObjects succeeds");
        match value {
            Value::Array(entries) => {
                assert_eq!(
                    entries,
                    vec![
                        object_reference_value(second),
                        object_reference_value(first)
                    ]
                );
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn get_ocf_returns_object_mask() {
        let ocf_mask = ocf::AVAILABLE | ocf::ALIVE;
        let object_id = ObjectId::new(1);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            object_id,
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_ocf(ocf_mask)]);

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            100,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_alive(true)
        .with_base_graphics(None)
        .with_ocf(ocf_mask);

        let (result, _) = with_effect_context(Some(object_context), &[], world, 2, || get_ocf(&[]));
        let value = result.expect("GetOCF succeeds");
        let Value::Int(raw) = value else {
            panic!("expected integer mask, got {value:?}");
        };
        let mask = raw as u32;
        assert_eq!(mask & ocf_mask, ocf_mask);
        assert_ne!(mask & ocf::NORMAL, 0);
        assert_ne!(mask & ocf::NOT_CONTAINED, 0);
    }

    #[test]
    fn set_graphics_records_overlay_update() {
        let object_id = ObjectId::new(42);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(Vec::new())
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_graphics(&[
                    Value::String("Default".into()),
                    Value::Nil,
                    Value::String("Clonk".into()),
                    Value::Int(1),
                    Value::Int(GraphicsOverlayMode::Action as i32),
                    Value::String("Walk".into()),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        assert_eq!(overlays.len(), 1);
        let overlay = &overlays[0];
        assert_eq!(overlay.id, 1);
        assert_eq!(overlay.mode, GraphicsOverlayMode::Action);
        assert_eq!(overlay.definition.as_deref(), Some("Clonk"));
        assert_eq!(overlay.action.as_deref(), Some("Walk"));
    }

    #[test]
    fn set_graphics_removes_overlay_when_definition_missing() {
        let object_id = ObjectId::new(7);
        let overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("Clonk".into()));
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![overlay])
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_graphics(&[
                    Value::String("Default".into()),
                    Value::Nil,
                    Value::Nil,
                    Value::Int(1),
                    Value::Int(GraphicsOverlayMode::Action as i32),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        assert!(overlays.is_empty());
    }

    #[test]
    fn set_graphics_updates_base_graphics() {
        let object_id = ObjectId::new(11);
        let definitions = {
            let mut map = HashMap::new();
            map.insert("CLON".to_string(), DefinitionMetadata::default());
            map.insert("BRIK".to_string(), DefinitionMetadata::default());
            map
        };
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                object_id,
                "CLON",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            100,
            false,
        );

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );

        let (result, outcome) = with_effect_context(
            Some(object_context.with_base_graphics(None)),
            &[],
            world,
            100,
            || {
                set_graphics(&[
                    Value::String("Alt".into()),
                    Value::Nil,
                    Value::String("BRIK".into()),
                    Value::Int(0),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let base = update
            .base_graphics
            .expect("base graphics update expected")
            .expect("base graphics set");
        assert_eq!(base.definition, "BRIK");
        assert_eq!(base.graphics_name.as_deref(), Some("Alt"));
        assert_eq!(base.blit_mode, 0);
    }

    #[test]
    fn set_graphics_clears_base_graphics_when_nil() {
        let object_id = ObjectId::new(12);
        let definitions = {
            let mut map = HashMap::new();
            map.insert("CLON".to_string(), DefinitionMetadata::default());
            map
        };
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                object_id,
                "CLON",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            100,
            false,
        );

        let base = ObjectBaseGraphics {
            definition: "CLON".to_string(),
            graphics_name: Some("Alt".into()),
            blit_mode: 0,
        };

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_base_graphics(Some(base));

        let (result, outcome) = with_effect_context(Some(object_context), &[], world, 100, || {
            set_graphics(&[Value::Nil, Value::Nil, Value::Nil, Value::Int(0)])
        });

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let base = update.base_graphics.expect("base graphics update expected");
        assert!(base.is_none());
    }

    #[test]
    fn set_obj_draw_transform_updates_object_transform() {
        let object_id = ObjectId::new(1);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );

        let object_context = object_context.with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_obj_draw_transform(&[
                    Value::Int(2000),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1500),
                    Value::Int(0),
                ])
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform succeeds"),
            Value::Bool(true)
        );
        let update = outcome.object_update.expect("object update expected");
        let transform = update
            .draw_transform
            .expect("transform update expected")
            .expect("transform set");
        assert!((transform.scale_x - 2.0).abs() < f32::EPSILON);
        assert!((transform.scale_y - 1.5).abs() < f32::EPSILON);
        assert!(transform.offset_x.abs() < f32::EPSILON);
        assert!(transform.offset_y.abs() < f32::EPSILON);
    }

    #[test]
    fn set_obj_draw_transform_updates_overlay_transform() {
        let object_id = ObjectId::new(5);
        let overlay = ObjectGraphicsOverlay::new(2, GraphicsOverlayMode::Base);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![overlay])
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_obj_draw_transform(&[
                    Value::Int(1000),
                    Value::Int(0),
                    Value::Int(500),
                    Value::Int(0),
                    Value::Int(1000),
                    Value::Int(-250),
                    Value::Proplist({
                        let mut map = HashMap::new();
                        map.insert("id".into(), Value::Int(object_id.as_u64() as i32));
                        map
                    }),
                    Value::Int(2),
                ])
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform succeeds"),
            Value::Bool(true)
        );
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        let overlay = overlays
            .iter()
            .find(|overlay| overlay.id == 2)
            .expect("overlay present");
        let transform = overlay.transform.expect("overlay transform set");
        assert!((transform.offset_x - 0.5).abs() < f32::EPSILON);
        assert!((transform.offset_y + 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn object_count_returns_number_of_matches() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(30),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(31),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(10, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [Value::String("Dummy".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || object_count(&args));
        let value = result.expect("ObjectCount succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn object_count_honours_owner_filter() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(40),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(41),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                2,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::String("Dummy".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            // iOwner is ObjectCount's 10th parameter (C4Script.cpp:2085).
            Value::Int(2),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || object_count(&args));
        let value = result.expect("ObjectCount owner succeeds");
        assert_eq!(value, Value::Int(1));
    }

    #[test]
    fn find_objects_returns_all_matches_in_order() {
        let container = ObjectId::new(40);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container,
                "Container",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(41),
                "Item",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(3, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
            HostWorldObject::new(
                ObjectId::new(42),
                "Item",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
        ]);
        let args = [
            Value::String("Item".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(ANY_CONTAINER_SENTINEL),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_objects(&args));
        let value = result.expect("FindObjects succeeds");
        match value {
            Value::Array(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0], object_reference_value(ObjectId::new(41)));
                assert_eq!(entries[1], object_reference_value(ObjectId::new(42)));
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    proptest! {
        #[test]
        fn do_energy_sequence_clamps_within_bounds(deltas in proptest::collection::vec(-200..=200i32, 0..16)) {
            let start_energy = DEFAULT_MAX_ENERGY;
            let expected = expected_energy_after_sequence(start_energy, &deltas);

            let sequence = deltas.clone();
            let (result, outcome) = with_object_host_context(move || {
                for delta in sequence.iter().copied() {
                    let value = do_energy(&[Value::Int(delta)])?;
                    match value {
                        Value::Bool(true) => {}
                        Value::Bool(false) => {
                            return Err(RuntimeError::new("DoEnergy rejected update"));
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "DoEnergy returned unexpected value: {}",
                                other.type_name()
                            )));
                        }
                    }
                }
                Ok(Value::Nil)
            });

            prop_assert!(result.is_ok());

            let final_energy = outcome
                .object_update
                .and_then(|update| update.energy)
                .unwrap_or(start_energy);

            prop_assert_eq!(final_energy, expected);
        }
    }

    // C4Object::DoEnergy model (C4Object.cpp:1345-1364): percent deltas
    // scale by C4MaxPhysical/100; the harness object has no physical, so
    // only the zero floor applies (real content always clamps to
    // GetPhysical()->Energy).
    fn expected_energy_after_sequence(start: i32, deltas: &[i32]) -> i32 {
        let mut energy = start;
        for &delta in deltas {
            energy = energy
                .saturating_add(delta.saturating_mul(LEGACY_MAX_PHYSICAL / 100))
                .max(0);
        }
        energy
    }

    #[test]
    fn add_global_effect_records_global_command() {
        let (result, outcome) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(120)])
            });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert!(outcome.object.is_empty());
        assert_eq!(outcome.global.len(), 1);
        match &outcome.global[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 120);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn global_effect_queries_use_context_view() {
        let (result, _) = with_effect_context(
            None,
            &[],
            HostWorldContext::default(),
            1,
            || -> Result<Value, RuntimeError> {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(90)])?;
                get_effect(&[
                    Value::String("Glow".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(1),
                ])
            },
        );

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn remove_global_effect_handles_missing() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            remove_effect(&[Value::Nil, Value::Nil, Value::Int(0)])
        });

        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }
}
