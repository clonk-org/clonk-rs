use std::cell::RefCell;
use std::collections::{hash_map::Entry, BTreeMap, HashMap, HashSet};
use std::convert::TryFrom;
use std::rc::Rc;

use crate::command::{
    CommandData, CommandId, CommandMode, CommandOperation, CommandRequest, MAX_COMMAND_STACK,
};
use crate::effect::{EffectCommand, EffectState, EffectVarValue};
use crate::math::{fixtoi_prec, integer_distance, itofix_prec, C4Fixed, FixedVec2};
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
    ObjectGraphicsOverlay, ObjectId, ObjectStatus, ObjectUpdate, ObjectVertex, ParticleCommand,
    ParticleConfig, ParticleLayer, ParticleScope, PathFinder, PhysicsSettings, PlayerState,
    QueuedCommand, SpawnConfig, TransferZoneCommand, TransferZoneRect, TransferZoneState, Vector2,
    CATEGORY_SORT_LIMIT, CNAT_BOTTOM, CNAT_CENTER, CNAT_LEFT, CNAT_NO_COLLISION, CNAT_RIGHT,
    CNAT_TOP, DEFAULT_CATEGORY, FULL_CON, OWNER_NONE,
};
use lc_script::{Engine as ScriptEngine, RuntimeError, Value};
use std::mem;
use tracing::{debug, info};

thread_local! {
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
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DefinitionMetadata {
    pub category: i32,
    pub ocf_base: u32,
    pub crew_member: bool,
    #[allow(dead_code)]
    pub value: i32,
    #[allow(dead_code)]
    pub mass: i32,
    pub constructable: bool,
    pub shape: Option<DefinitionRect>,
    pub construction_offset: i32,
    #[allow(dead_code)]
    pub basement: i32,
}

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
        }
    }

    pub(crate) fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub(crate) fn with_ocf(mut self, ocf: u32) -> Self {
        self.ocf = ocf;
        self
    }

    pub fn alive(&self) -> bool {
        self.alive
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

#[derive(Debug, Clone)]
pub(crate) struct HostWorldContext {
    objects: Rc<HashMap<ObjectId, HostWorldObject>>,
    order: Rc<Vec<ObjectId>>,
    landscape: Option<Rc<Landscape>>,
    definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
    sectors: Option<Rc<SectorMap>>,
    transfer_zones: Rc<Vec<TransferZoneState>>,
    players: Rc<HashMap<i32, PlayerState>>,
    player_order: Rc<Vec<i32>>,
    crew_selection: Rc<HashMap<i32, CrewSelectionState>>,
    next_object_id: u64,
    team_home_base_rule: bool,
}

impl Default for HostWorldContext {
    fn default() -> Self {
        Self {
            objects: Rc::new(HashMap::new()),
            order: Rc::new(Vec::new()),
            landscape: None,
            definitions: Rc::new(HashMap::new()),
            sectors: None,
            transfer_zones: Rc::new(Vec::new()),
            players: Rc::new(HashMap::new()),
            player_order: Rc::new(Vec::new()),
            crew_selection: Rc::new(HashMap::new()),
            next_object_id: 1,
            team_home_base_rule: false,
        }
    }
}

impl HostWorldContext {
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
        let map = objects.into_iter().collect::<Vec<HostWorldObject>>();
        let sectors = landscape
            .as_ref()
            .map(|landscape| Rc::new(build_host_sector_map(&map, &definitions, landscape)));
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
            definitions: Rc::new(definitions),
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
        }
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

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sectors.as_ref().map(|sectors| {
            let area = sectors.area(rect);
            sectors.object_ids_in_area(&area)
        })
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sectors.as_ref().map(|sectors| {
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

fn build_host_sector_map(
    objects: &[HostWorldObject],
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
    landscape: &Landscape,
) -> SectorMap {
    let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
    let height = landscape.estimated_height();
    let mut sectors = SectorMap::new(width, height);
    sectors.rebuild(
        objects
            .iter()
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
}

impl WorldAccessor for EffectHostContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get_world_object(id)
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.world_object_ids()
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
    let mut map = HashMap::new();
    map.insert("id".into(), Value::Int(truncate_to_i32(id.as_u64())));
    Value::Proplist(map)
}

fn parse_object_reference_argument(
    value: &Value,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    match value {
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id >= 0 => Ok(Some(ObjectId::new(*id as u64))),
            _ => Ok(None),
        },
        Value::Nil => Ok(None),
        Value::Int(id) if *id == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{}: expected proplist, nil, or 0 for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn value_to_i32(value: &Value, function: &str, parameter: &str) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(int) => Ok(*int),
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
    let filter = parse_player_type_filter(args.get(0), "GetPlayerCount")?;
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
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetPlayerByIndex expects at least 1 argument: index",
        ));
    }
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetPlayerByIndex expects at most 2 arguments: index and type",
        ));
    }
    let index = value_to_i32(&args[0], "GetPlayerByIndex", "index")?;
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
        Some(Value::Proplist(map)) => match map.get("id") {
            Some(Value::Int(id)) if *id > 0 => {
                Ok(ContainerFilter::Exact(ObjectId::new(*id as u64)))
            }
            _ => Err(RuntimeError::new(format!(
                "{}: expected object reference proplist for container",
                function
            ))),
        },
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected object reference or container sentinel, got {}",
            function,
            other.type_name()
        ))),
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
    fn parse(args: &[Value]) -> Result<Self, RuntimeError> {
        if args.len() > 12 {
            return Err(RuntimeError::new(
                "FindObject: expected at most 12 arguments",
            ));
        }

        let definition = parse_definition_argument(args.get(0), "FindObject")?;
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
            return dx >= 0 && dx <= self.width - 1 && dy >= 0 && dy <= self.height - 1;
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

fn energy_to_script_value(energy: i32) -> i32 {
    if energy <= DEFAULT_MAX_ENERGY {
        energy
    } else {
        let numerator = (energy as i64) * 100;
        let denominator = LEGACY_MAX_PHYSICAL as i64;
        (numerator / denominator) as i32
    }
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

fn compute_vertex_contact(
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

fn resolve_vertices<'a>(
    context: &'a EffectHostContext,
    target: Option<ObjectId>,
) -> Option<(Vector2, &'a [ObjectVertex])> {
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

/// GameCallEx - Broadcast a function call to Goal/Rule/Environment objects and scenario script
///
/// C++ signature: GameCallEx(string szFunction, any par0-par8)
///
/// Broadcasts an optional function call (prefixed with ~) to:
/// 1. All objects with category Goal, Rule, or Environment
/// 2. The scenario script
///
/// Returns the result from the scenario script call, or nil if no handlers exist.
///
/// TODO: Full implementation requires:
/// - Iterating through world objects filtered by category
/// - Calling script functions on those objects
/// - Calling scenario script function
/// For now, returns nil to unblock integration tests.
fn game_call_ex(args: &[Value]) -> Result<Value, RuntimeError> {
    // Validate we have at least a function name
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GameCallEx requires at least a function name",
        ));
    }

    let _function_name = match &args[0] {
        Value::String(s) => s.as_str(),
        other => {
            return Err(RuntimeError::new(format!(
                "GameCallEx function name must be string, got {}",
                other.type_name()
            )))
        }
    };

    // C++ implementation prefixes function name with ~ for failsafe calling,
    // then broadcasts to Goal/Rule/Environment objects and scenario script.
    //
    // For now, return nil. The ~ prefix means the call is optional - if no
    // handlers exist, it's not an error.
    //
    // Full implementation would require architectural changes to allow
    // host functions to iterate objects and call script functions on them.

    Ok(Value::Nil)
}

pub fn register_host_functions(script: &mut ScriptEngine) {
    script.register_host_function("AddEffect", add_effect);
    script.register_host_function("RemoveEffect", remove_effect);
    script.register_host_function("GetEffect", get_effect);
    script.register_host_function("GetEffectCount", get_effect_count);
    script.register_host_function("EffectVar", effect_var);
    script.register_host_function("GetPlayerCount", get_player_count);
    script.register_host_function("GetPlayerByIndex", get_player_by_index);
    script.register_host_function("GetPlayerName", get_player_name);
    script.register_host_function("GetPlayerTeam", get_player_team);
    script.register_host_function("GetPlayerType", get_player_type);
    script.register_host_function("GetPlayerID", get_player_id);
    script.register_host_function("GetWealth", get_wealth);
    script.register_host_function("GetScore", get_score);
    script.register_host_function("GetPlrValue", get_plr_value);
    script.register_host_function("GetPlrValueGain", get_plr_value_gain);
    script.register_host_function("GetPlrKnowledge", get_plr_knowledge);
    script.register_host_function("GetCrew", get_crew);
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
    script.register_host_function("GetActTime", get_act_time);
    script.register_host_function("GetPhase", get_phase);
    script.register_host_function("SetPhase", set_phase);
    script.register_host_function("GetProcedure", get_procedure);
    script.register_host_function("SetActionTargets", set_action_targets);
    script.register_host_function("GetActionTarget", get_action_target);
    script.register_host_function("GetVertexNum", get_vertex_num);
    script.register_host_function("GetVertex", get_vertex);
    script.register_host_function("GetVertexContact", get_vertex_contact);
    script.register_host_function("GetContact", get_contact);
    script.register_host_function("PathFree", path_free);
    script.register_host_function("GetPath", get_path);
    script.register_host_function("SetTransferZone", set_transfer_zone);
    script.register_host_function("DigFree", dig_free);
    script.register_host_function("DigFreeRect", dig_free_rect);
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
    script.register_host_function("FindObjects", find_objects);
    script.register_host_function("ObjectCount", object_count);
    script.register_host_function("ObjectDistance", object_distance);
    script.register_host_function("GetX", get_x);
    script.register_host_function("GetY", get_y);
    script.register_host_function("GetID", get_id);
    script.register_host_function("SetPosition", set_position);
    script.register_host_function("CreateObject", create_object);
    script.register_host_function("CreateConstruction", create_construction);
    script.register_host_function("CreateParticle", create_particle);
    script.register_host_function("ClearParticles", clear_particles);
    script.register_host_function("CustomMessage", custom_message);
    script.register_host_function("Message", message);
    script.register_host_function("PlayerMessage", player_message);
    script.register_host_function("AddMessage", add_message);
    script.register_host_function("PlrMessage", plr_message);
    script.register_host_function("Log", log_message);
    script.register_host_function("DebugLog", debug_log_message);
    script.register_host_function("GameOver", game_over);
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
    script.register_host_function("SetAlive", set_alive);
    script.register_host_function("GetAlive", get_alive);
    script.register_host_function("SetOwner", set_owner);
    script.register_host_function("GetOwner", get_owner);
    script.register_host_function("SetObjectStatus", set_object_status);
    script.register_host_function("GetObjectStatus", get_object_status);
    script.register_host_function("GetOCF", get_ocf);
    script.register_host_function("SetGraphics", set_graphics);
    script.register_host_function("SetObjDrawTransform", set_obj_draw_transform);
    script.register_host_function("SetObjDrawTransform2", set_obj_draw_transform2);
    script.register_host_function("RemoveObject", remove_object);
    script.register_host_function("GetEnergy", get_energy);
    script.register_host_function("DoEnergy", do_energy);
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
    let map = match args.get(0) {
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
    let map = match args.get(0) {
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
    if size < 0 || size > LEGACY_MAX_ARRAY_SIZE {
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
    pub container: Option<ObjectId>,
    pub status: ObjectStatus,
    pub energy: i32,
    pub damage: i32,
    pub alive: bool,
    pub owner: i32,
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
            container,
            status,
            energy,
            damage,
            construction: construction.clamp(0, FULL_CON),
            alive: true,
            owner,
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
        }
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
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
        .unwrap_or_else(AudioRegistry::new);
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
    BlastCircle {
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    },
    ShakeCircle {
        center: Vector2,
        radius: i32,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct EffectContextOutcome {
    pub object: Vec<EffectCommand>,
    pub global: Vec<EffectCommand>,
    pub object_update: Option<ObjectUpdate>,
    pub object_commands: Vec<QueuedCommand>,
    pub command_operations: Vec<CommandOperation>,
    pub destroy_object: bool,
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
    pub next_object_id: u64,
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
        next_object_id: u64,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
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
            next_object_id,
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
            next_object_id,
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

fn add_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "AddEffect expects at least 2 arguments: name and state",
        ));
    }

    let name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Int(0)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected proplist for object state, got {}",
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

    let interval = match args.get(3) {
        Some(Value::Int(value)) if *value > 0 => *value,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "AddEffect: interval must be > 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 1,
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
            Value::Proplist(_) | Value::Nil => {
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
        ctx.add_effect(effect)
    })?;

    Ok(Value::Int(identifier))
}

fn remove_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "RemoveEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        Value::Int(value) if *value == 0 => None,
        other => {
            return Err(RuntimeError::new(format!(
                "RemoveEffect: expected string, nil, or 0 for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected proplist for object state, got {}",
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
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.as_str()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

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

    let mut match_index = 0;
    for effect in &effects {
        if let Some(filter) = name_filter {
            if effect.name != filter {
                continue;
            }
        }

        if let Some(limit) = max_priority {
            if effect.priority.abs() > limit {
                continue;
            }
        }

        if match_index == desired_index {
            return Ok(match query {
                0 => {
                    let identifier = match_index.saturating_add(1);
                    let id = i32::try_from(identifier).unwrap_or(i32::MAX);
                    Value::Int(id)
                }
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
            });
        }

        match_index += 1;
    }

    Ok(Value::Nil)
}

fn get_effect_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffectCount expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.as_str()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffectCount: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

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
                if effect.name != filter {
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

    let effects = extract_effects_from_state(&args[1])?;
    if effect_number == 0 || effect_number > effects.len() {
        return Ok(Value::Nil);
    }
    let effect = &effects[effect_number - 1];
    let value = effect.var(var_index);
    Ok(effect_var_to_value(&value))
}

fn random(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "Random expects exactly 1 argument: upper exclusive bound",
        ));
    }

    let range = match &args[0] {
        Value::Int(value) => *value,
        other => {
            return Err(RuntimeError::new(format!(
                "Random: expected int for range, got {}",
                other.type_name()
            )))
        }
    };

    if range <= 0 {
        return Ok(Value::Int(0));
    }

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

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetWind requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.wind_force()))
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
    if let Some(arg) = args.get(0) {
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

fn get_con(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetCon expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
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
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
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

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
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

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
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
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.adjust_energy(change, exact);
        Ok(Value::Bool(true))
    })
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
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
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

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
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

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoDamage requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.adjust_damage(change);
        Ok(Value::Bool(true))
    })
}

fn set_action(args: &[Value]) -> Result<Value, RuntimeError> {
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

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    let mut phase: Option<i32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                phase = Some(*value);
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "SetAction: expected int or nil for phase, got {}",
                    arg.type_name()
                )))
            }
        }
    }

    let mut ticks: Option<u32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) if *value >= 0 => {
                ticks = Some(*value as u32);
                index += 1;
            }
            Value::Int(_) => {
                return Err(RuntimeError::new(
                    "SetAction: ticks must be >= 0 when provided",
                ));
            }
            Value::Nil => {
                index += 1;
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "SetAction: expected int or nil for ticks, got {}",
                    arg.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetAction: additional arguments are not supported",
        ));
    }

    let name = match action_name {
        Some(name) => name,
        None => return Ok(Value::Bool(false)),
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetAction requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

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
        if let Some(phase) = phase {
            update.set_phase(phase);
        }

        if let Some(ticks) = ticks {
            object.set_action_ticks(ticks);
        } else if changed_action {
            object.reset_action_ticks();
        }

        let procedure_changed = object.update_effective_action(&name);
        if procedure_changed {
            object.reset_action_data();
        }

        Ok(Value::Bool(true))
    })
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

fn get_action(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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

        object.set_action_phase(phase);
        Ok(Value::Bool(true))
    })
}

fn get_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetContact: requires a vertex index argument",
        ));
    }

    let vertex_index = value_to_i32(&args[0], "GetContact", "index")?;
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
        target_id = parse_object_reference_argument(arg, "GetContact", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetContact: additional arguments are not supported",
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

        let clear = landscape.path_is_clear(Vector2::new(x1, y1), Vector2::new(x2, y2));
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
    if args.len() != 2 {
        return Err(RuntimeError::new(format!(
            "{function} expects 2 arguments: x, y"
        )));
    }

    let local_x = value_to_i32(&args[0], function, "x")?;
    let local_y = value_to_i32(&args[1], function, "y")?;

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
            LandscapeQuery::SemiSolid => landscape.is_solid_at(x, y),
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
    if args.len() != 2 {
        return Err(RuntimeError::new("GetMaterial expects 2 arguments: x, y"));
    }

    let local_x = value_to_i32(&args[0], "GetMaterial", "x")?;
    let local_y = value_to_i32(&args[1], "GetMaterial", "y")?;

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

fn find_object(args: &[Value]) -> Result<Value, RuntimeError> {
    let params = FindObjectParams::parse(args)?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
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
    let params = FindObjectParams::parse(args)?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(0)),
        };
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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_direction(direction);
        Ok(Value::Bool(true))
    })
}

fn get_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
        parse_object_reference_argument(args.get(0).unwrap_or(&Value::Nil), "GetR", "target")?;

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetCommand expects at least 1 argument: command name",
        ));
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
    if let Some(arg) = args.get(0) {
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
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
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
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
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

        // C++ SetXDir/SetYDir set xdir/ydir = itofix(value, prec) (default
        // precision 10), storing fractional `C4Fixed` velocity. `C4Script.cpp:697`.
        let mut fixed = object.fixed_velocity();
        component.assign_fixed(
            &mut fixed,
            itofix_prec(value, normalise_precision(precision)),
        );
        object.set_fixed_velocity(fixed);
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
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
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
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
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
    if let Some(arg) = args.get(0) {
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

    let definition = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateObject: expected string for definition, got {}",
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

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateObject requires an active engine context"))?;

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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

        let id = context.allocate_object_id();

        let spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
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
        .with_ocf(preview_ocf);

        context.register_spawn(spawn, preview);
        Ok(object_reference_value(id))
    })
}

fn create_construction(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateConstruction expects at least 1 argument: definition",
        ));
    }

    let definition = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateConstruction: expected string for definition, got {}",
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

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: true,
                shape: None,
                construction_offset: 0,
                basement: 0,
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

        let spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
            .with_construction(construction_value)
            .with_id(id);

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
        .with_ocf(preview_ocf);

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
    if let Some(arg) = args.get(0) {
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

    let index = match args.get(0) {
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

    let definition = parse_definition_argument(args.get(0), "ContentsCount")?;
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

    let target_value = args.get(0).unwrap_or(&Value::Nil);
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

    let target_value = args.get(0).unwrap_or(&Value::Nil);
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
    if let Some(arg) = args.get(0) {
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

fn get_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetAlive expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
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
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "RemoveObject", "target")?;
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

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

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
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

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

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
            }
            Value::Nil => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "GetObjectStatus: expected proplist or nil for target, got {}",
                    other.type_name()
                )))
            }
        }
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

        Ok(Value::Int(object.status().to_script_value()))
    })
}

fn snapshot_effects_from_context(scope: EffectScope) -> Option<Vec<EffectState>> {
    HOST_CONTEXT.with(|cell| cell.borrow().as_ref().and_then(|ctx| ctx.snapshot(scope)))
}

fn determine_scope_from_state(value: &Value) -> Result<EffectScope, RuntimeError> {
    match value {
        Value::Proplist(_) => Ok(EffectScope::Object),
        Value::Nil => Ok(EffectScope::Global),
        Value::Int(id) if *id == 0 => Ok(EffectScope::Global),
        other => Err(RuntimeError::new(format!(
            "effect host functions expected proplist, nil, or 0 for state, got {}",
            other.type_name()
        ))),
    }
}

fn extract_effects_from_state(state: &Value) -> Result<Vec<EffectState>, RuntimeError> {
    let map = match state {
        Value::Proplist(map) => map,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected proplist or nil for state, got {}",
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
    let mut map = HashMap::with_capacity(5);
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
        let vars = effect
            .vars()
            .iter()
            .map(|value| effect_var_to_value(value))
            .collect();
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
        Value::Array(entries) => {
            let vars = entries
                .iter()
                .map(|entry| value_to_effect_var(entry))
                .collect();
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
        EffectVarValue::Array(entries) => {
            let vars = entries
                .iter()
                .map(|entry| effect_var_to_value(entry))
                .collect();
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
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) => Ok(Some(*id)),
            _ => Err(RuntimeError::new(
                "AddEffect: command target proplist must contain int `id`",
            )),
        },
        Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected proplist, nil, or 0 for command target, got {}",
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
            .unwrap_or_else(AudioRegistry::new)
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

struct EffectHostContext {
    object: Option<ObjectScopeContext>,
    global: Option<EffectScopeContext>,
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
    game_over_triggered: bool,
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
                container,
                status,
                energy,
                damage,
                construction,
                alive,
                owner,
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
            } = ctx;
            ObjectScopeContext::new(
                id,
                container,
                status,
                energy,
                damage,
                construction,
                alive,
                owner,
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
            )
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
            game_over_triggered,
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
        if let Some(object) = self.pending_objects.get(&id) {
            Some(object.clone())
        } else {
            self.world.get(id).cloned()
        }
    }

    fn world_object_ids(&self) -> Vec<ObjectId> {
        let mut ids = self.world.object_ids().to_vec();
        ids.extend(self.pending_order.iter().copied());
        ids
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
            self.next_object_id,
        );
        outcome.particles = self.pending_particles;
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

    fn add_effect(&mut self, mut effect: EffectState) -> i32 {
        if effect.interval <= 0 {
            effect.interval = 1;
        }
        if effect.timer < 0 {
            effect.timer = 0;
        }
        if effect.interval > 0 && effect.timer >= effect.interval {
            effect.timer %= effect.interval;
        }

        if let Some(index) = self
            .effects
            .iter()
            .position(|existing| existing.name == effect.name)
        {
            self.effects.remove(index);
        }

        let mut insert_pos = 0;
        while insert_pos < self.effects.len() && self.effects[insert_pos].priority > effect.priority
        {
            insert_pos += 1;
        }

        self.effects.insert(insert_pos, effect.clone());
        self.commands.push(EffectCommand::add(effect));
        (insert_pos + 1) as i32
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
        let index = effect_number - 1;
        if index >= self.effects.len() {
            return None;
        }
        let effect = &mut self.effects[index];
        if let Some(value) = new_value {
            effect.set_var(var_index, value);
            let updated = effect.clone();
            self.commands.push(EffectCommand::add(updated));
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
                if effect.name == name {
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
            if self.effects.is_empty() {
                None
            } else if index == 0 {
                Some(0)
            } else {
                let effect_number = index.saturating_sub(1);
                if effect_number < self.effects.len() {
                    Some(effect_number)
                } else {
                    None
                }
            }
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
    max_energy: i32,
    current_owner: i32,
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
        owner: i32,
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
    ) -> Self {
        let blocks_other_actions = action_library.blocks_other_actions(&action_name);
        let max_energy = energy.max(DEFAULT_MAX_ENERGY);
        let clamped_damage = damage.max(0);
        let clamped_construction = construction.clamp(0, FULL_CON);
        Self {
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
            max_energy,
            current_owner: owner,
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
        }
    }

    fn id(&self) -> ObjectId {
        self.id
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
    }

    fn alive(&self) -> bool {
        self.pending_update.alive.unwrap_or(self.current_alive)
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

    #[allow(dead_code)]
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
        if energy > self.max_energy {
            self.max_energy = energy;
        }
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

    fn adjust_energy(&mut self, delta: i32, _exact: bool) -> i32 {
        let mut next = self.energy().saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        if next > self.max_energy {
            next = self.max_energy;
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
        "AddCommand",
        "AddEffect",
        "AddMessage",
        "AppendCommand",
        "BlastFree",
        "BoundBy",
        "ClearParticles",
        "Contained",
        "Contents",
        "ContentsCount",
        "Cos",
        "CreateArray",
        "CreateConstruction",
        "CreateObject",
        "CreateParticle",
        "CustomMessage",
        "DebugLog",
        "DigFree",
        "DigFreeRect",
        "DoCon",
        "DoDamage",
        "DoEnergy",
        "DoHomebaseMaterial",
        "DoHomebaseProduction",
        "EffectVar",
        "FindContents",
        "FindObject",
        "FindObjects",
        "FindOtherContents",
        "Format",
        "GBackLiquid",
        "GBackSemiSolid",
        "GBackSky",
        "GBackSolid",
        "GameCallEx",
        "GameOver",
        "GetActTime",
        "GetAction",
        "GetActionData",
        "GetActionTarget",
        "GetAlive",
        "GetCategory",
        "GetClimate",
        "GetComDir",
        "GetCon",
        "GetContact",
        "GetCrew",
        "GetCrewCount",
        "GetCursor",
        "GetDir",
        "GetEffect",
        "GetEffectCount",
        "GetEnergy",
        "GetGravity",
        "GetHomebaseMaterial",
        "GetHomebaseProduction",
        "GetID",
        "GetIndexOf",
        "GetKeys",
        "GetLength",
        "GetMaterial",
        "GetOCF",
        "GetObjectStatus",
        "GetOwner",
        "GetPath",
        "GetPhase",
        "GetPlayerByIndex",
        "GetPlayerCount",
        "GetPlayerID",
        "GetPlayerName",
        "GetPlayerTeam",
        "GetPlayerType",
        "GetPlrKnowledge",
        "GetPlrValue",
        "GetPlrValueGain",
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
        "GetWealth",
        "GetWind",
        "GetX",
        "GetXDir",
        "GetY",
        "GetYDir",
        "Log",
        "Max",
        "Message",
        "Min",
        "ObjectCount",
        "ObjectDistance",
        "PathFree",
        "PlayerMessage",
        "PlrMessage",
        "Pow",
        "Random",
        "RemoveEffect",
        "RemoveObject",
        "SetAction",
        "SetActionData",
        "SetActionTargets",
        "SetAlive",
        "SetBridgeActionData",
        "SetCategory",
        "SetClimate",
        "SetComDir",
        "SetCommand",
        "SetDir",
        "SetGraphics",
        "SetGravity",
        "SetObjDrawTransform",
        "SetObjDrawTransform2",
        "SetObjectStatus",
        "SetOwner",
        "SetPhase",
        "SetPlrKnowledge",
        "SetPosition",
        "SetR",
        "SetRDir",
        "SetTemperature",
        "SetTransferZone",
        "SetWind",
        "SetXDir",
        "SetYDir",
        "ShakeFree",
        "Sin",
        "Sound",
        "SoundLevel",
        "Sqrt",
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
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
                    category: 0x1,
                    ocf_base: 0,
                    crew_member: false,
                    value: 0,
                    mass: 0,
                    constructable: false,
                    shape: None,
                    construction_offset: 0,
                    basement: 0,
                },
            ),
            (
                "STON".to_string(),
                DefinitionMetadata {
                    category: 0x2,
                    ocf_base: 0,
                    crew_member: false,
                    value: 0,
                    mass: 0,
                    constructable: false,
                    shape: None,
                    construction_offset: 0,
                    basement: 0,
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
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
                category: 0x1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
                category: 1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
                category: 1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
                category: 1,
                ocf_base: 0,
                crew_member: false,
                value: 0,
                mass: 0,
                constructable: false,
                shape: None,
                construction_offset: 0,
                basement: 0,
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
    fn random_zero_or_negative_range_short_circuits() {
        let zero = random(&[Value::Int(0)]).expect("zero range succeeds");
        let negative = random(&[Value::Int(-3)]).expect("negative range succeeds");
        assert_eq!(zero, Value::Int(0));
        assert_eq!(negative, Value::Int(0));
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
                set_action(&[
                    Value::String("Idle".into()),
                    Value::Nil,
                    Value::Nil,
                    Value::Int(7),
                ])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(7));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(7));
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
            || get_contact(&[Value::Int(-1)]),
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(12)));
            }
            other => panic!("expected proplist, got {:?}", other),
        }

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
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action_target(&[Value::Int(0), Value::Proplist(target)])
        });

        let value = result.expect("GetActionTarget succeeds");
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(77)));
            }
            other => panic!("expected proplist, got {:?}", other),
        }
    }

    #[test]
    fn get_action_target_returns_nil_for_out_of_range_index() {
        let value = with_object_host_context(|| get_action_target(&[Value::Int(2)]))
            .0
            .expect("GetActionTarget succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn set_dir_records_direction_update() {
        let (result, outcome) = with_object_host_context(|| set_dir(&[Value::Int(1)]));
        let value = result.expect("SetDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn get_dir_observes_effective_direction() {
        let (result, outcome) = with_object_host_context(|| {
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
        let fixed = update.fixed_velocity.expect("fixed velocity recorded");
        assert_eq!(fixed.x, itofix_prec(15, 10));
        assert_eq!(fixed.x.val(), 98304);
        assert_eq!(fixed.y, C4Fixed::ZERO);
        // The whole-pixel mirror is derived via fixtoi(1.5) = 2.
        assert_eq!(update.velocity, Some(Vector2::new(2, 0)));
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
        let fixed = update.fixed_velocity.expect("fixed velocity recorded");
        assert_eq!(fixed.y, itofix_prec(5, 5));
        assert_eq!(fixed.y.val(), 65536);
        // Whole-pixel mirror is fixtoi(1.0) = 1.
        assert_eq!(update.velocity, Some(Vector2::new(0, 1)));
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
            ])?;

            let initial = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])?;
            assert_eq!(initial, Value::Int(3));

            let unset = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(unset, Value::Nil);

            let updated = effect_var(&[
                Value::Int(1),
                state.clone(),
                Value::Int(1),
                Value::String("beam".into()),
            ])?;
            assert_eq!(updated, Value::String("beam".into()));

            let reread = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(reread, Value::String("beam".into()));

            Ok(Value::Nil)
        });

        result.expect("EffectVar interactions succeed");
        assert_eq!(outcome.object.len(), 2);
        match &outcome.object[1] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.vars().len(), 2);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(3));
                assert_eq!(effect.vars()[1], EffectVarValue::String("beam".into()));
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
    fn set_action_respects_target_id() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(2));
        let args = vec![Value::String("Jump".into()), Value::Proplist(target_map)];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
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
    fn do_energy_applies_delta_and_clamps() {
        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(-25)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(75));

        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(50)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(100));
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
                75,
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
            33,
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
        assert_eq!(spawn.owner, OWNER_NONE);
        assert_eq!(spawn.id, Some(ObjectId::new(1)));
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_construction_registers_spawn_when_site_valid() {
        let landscape = Landscape::flat(64, 50);
        let definitions = HashMap::from([(
            "Workshop".to_string(),
            DefinitionMetadata {
                category: crate::CATEGORY_STRUCTURE,
                ocf_base: ocf::NORMAL,
                crew_member: false,
                value: 0,
                mass: 100,
                constructable: true,
                shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
                construction_offset: 0,
                basement: 0,
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
            category: crate::CATEGORY_STRUCTURE,
            ocf_base: ocf::NORMAL,
            crew_member: false,
            value: 0,
            mass: 100,
            constructable: true,
            shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
            construction_offset: 0,
            basement: 0,
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(
                    map.get("id"),
                    Some(&Value::Int(container_id.as_u64() as i32))
                );
            }
            other => panic!("expected proplist for container reference, got {other:?}"),
        }
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(item_id.as_u64() as i32)));
            }
            other => panic!("expected proplist result for contents, got {other:?}"),
        }
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(
                    map.get("id"),
                    Some(&Value::Int(attached_id.as_u64() as i32))
                );
            }
            other => panic!("expected proplist result for contents, got {other:?}"),
        }
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(gem_id.as_u64() as i32)));
            }
            other => panic!("expected proplist from FindContents, got {other:?}"),
        }
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
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(hammer_id.as_u64() as i32)));
            }
            other => panic!("expected proplist from FindOtherContents, got {other:?}"),
        }
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
    fn find_object_respects_owner_filter() {
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
        let value = result.expect("FindObject owner succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(11)));
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
            Value::Nil,
            Value::Nil,
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
        let map = match value {
            Value::Proplist(map) => map,
            other => panic!("expected object reference, got {other:?}"),
        };
        let id_value = map.get("id").expect("object reference contains id");
        let Value::Int(id) = id_value else {
            panic!("expected integer id, got {id_value:?}");
        };
        assert_eq!(*id, matching_id.as_u64() as i32);
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
    fn find_objects_sector_range_preserves_world_order() {
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
                        object_reference_value(first),
                        object_reference_value(second)
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
            Value::Nil,
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

    fn expected_energy_after_sequence(start: i32, deltas: &[i32]) -> i32 {
        let mut energy = start;
        for &delta in deltas {
            energy = energy.saturating_add(delta);
            if energy < 0 {
                energy = 0;
            } else if energy > DEFAULT_MAX_ENERGY {
                energy = DEFAULT_MAX_ENERGY;
            }
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
