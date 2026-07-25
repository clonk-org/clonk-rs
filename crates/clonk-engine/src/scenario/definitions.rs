//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

pub(in crate::scenario) fn collect_definitions_from_group<S: AsRef<str>>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    language_packs: &LanguagePacks,
    scenario: &Group,
    scenario_origin: Option<&str>,
    sound_effect_groups: &mut Vec<Group>,
    output: &mut Vec<CollectedDefinition>,
) -> Result<(), ScenarioError> {
    let mut primary_definition = false;
    // C4Def::Load diverts Particle.txt groups into C4ParticleDef before it
    // even attempts DefCore; they never become object definitions.
    if group.exists("Particle.txt") {
        // C4Def::Load marks particle groups as non-definitions, loads the
        // particle metadata, and then still runs the invalid-definition
        // LoadEffects path regardless of whether that metadata succeeded.
        sound_effect_groups.push(group.clone());
        match ResourceParticleDefinition::load(group) {
            Ok(definition) => output.push(CollectedDefinition::Particle(definition)),
            Err(error) => tracing::warn!(
                group = %group.root().display(),
                %error,
                "particle definition failed to load; skipping"
            ),
        }
    } else if group.exists("DefCore.txt") {
        // C4Def::Load checks SkipDefs immediately after DefCore, before
        // scripts, ActMap, graphics, sounds, or localized auxiliary data.
        // Probe the ID first so malformed data in a skipped definition is
        // never observed.
        let core = match ResourceDefCore::load(group) {
            Ok(core) => Some(core),
            Err(ResourceDefinitionError::DefCoreMissing) => {
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) if is_rejected_definition_error(&error) => {
                warn_rejected_definition(group, &error);
                // A failed C4DefCore::Load deliberately turns the group into
                // a pure sound container before C4DefList visits children.
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(core) = core {
            if !core.has_valid_id() {
                tracing::warn!(
                    id = %core.id,
                    group = %group.root().display(),
                    "skipping definition with invalid C4ID"
                );
                // NeededGfxMode is checked even after an invalid ID made the
                // definition unsuccessful. OLDGFX therefore suppresses the
                // otherwise intentional pure-sound fallback.
                if core.needed_gfx_mode != 2 {
                    sound_effect_groups.push(group.clone());
                }
            } else if skip_ids.contains(&core.id.to_ascii_uppercase()) {
                // C4Def::Load checks SkipDefs before the graphics-mode gate.
            } else if core.needed_gfx_mode == 2 {
                // C4DGFXMODE_OLDGFX is no longer supported. Native returns
                // false here without a dedicated diagnostic.
            } else {
                let components =
                    language_packs.component_groups(group, Some(scenario), scenario_origin);
                match ResourceDefinitionData::load_with_core_and_languages_and_components(
                    group,
                    core,
                    languages,
                    &components,
                ) {
                    Ok(resource) => {
                        if resource.graphics_image.is_none() {
                            warn_rejected_definition(
                                group,
                                &"required Graphics.png/Graphics.bmp is missing or invalid",
                            );
                        } else {
                            primary_definition = true;
                            // Valid definitions reach LoadEffects only after
                            // bitmap, portrait and ActMap/resource loading has
                            // succeeded. Retain the event before descending
                            // into child definitions.
                            sound_effect_groups.push(group.clone());
                            let mut definition =
                                scenario_definition_from_resource(resource, Some(group.clone()));
                            definition.script = localize_script_source_with_components(
                                &components,
                                &definition.script,
                                languages,
                            )?;
                            output.push(CollectedDefinition::Definition(definition));
                        }
                    }
                    Err(error) if is_rejected_definition_error(&error) => {
                        warn_rejected_definition(group, &error);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    } else {
        // Missing DefCore is the canonical pure `.c4d` sound-folder case.
        sound_effect_groups.push(group.clone());
    }

    // C4DefList::Load recursively visits only *.c4d children.
    for entry in group.entries()? {
        if !legacy_group_wildcard_match(b"*.c4d", &entry.name_bytes) {
            continue;
        }
        // FindNextEntry("*.c4d") also sees normal files and corrupt packed
        // entries. C4Group::OpenAsChild failure simply skips that candidate.
        let Ok(child) = group.open_child_entry_exact(&entry) else {
            continue;
        };
        // The recursive call omits fLoadSysGroups in C++, so its default true
        // applies even when only the scenario root suppressed System loading.
        collect_definitions_from_group(
            &child,
            true,
            skip_ids,
            languages,
            language_packs,
            scenario,
            scenario_origin,
            sound_effect_groups,
            output,
        )?;
    }

    // A non-primary definition root loads its System.c4g only AFTER all child
    // definitions (C4Def.cpp:927-968). Direct primary definitions suppress
    // their own System group, as does the scenario-file InitDefs pass.
    if !primary_definition && load_system_groups {
        if let Ok(system) = group.open_child(Path::new("System.c4g")) {
            let components =
                language_packs.component_groups(&system, Some(scenario), scenario_origin);
            if let Ok(sources) =
                load_system_scripts_with_components(&system, &components, languages)
            {
                output.push(CollectedDefinition::SystemScripts(sources));
            }
        }
    }
    Ok(())
}

fn is_rejected_definition_error(error: &ResourceDefinitionError) -> bool {
    matches!(
        error,
        ResourceDefinitionError::MissingDefCoreField(_)
            | ResourceDefinitionError::InvalidCategoryValue(_)
            | ResourceDefinitionError::DefCoreParse(_)
            | ResourceDefinitionError::ActMapParse(_)
            | ResourceDefinitionError::Graphics { .. }
            | ResourceDefinitionError::ColorByOwnerOverlay { .. }
    )
}

fn warn_rejected_definition(group: &Group, error: &impl fmt::Display) {
    tracing::warn!(
        group = %group.root().display(),
        error = %error,
        "definition failed to load; skipping"
    );
}

pub(in crate::scenario) fn scenario_definition_from_resource(
    resource: ResourceDefinitionData,
    source_group: Option<Group>,
) -> ScenarioDefinition {
    let script_name = source_group
        .as_ref()
        .map(|group| group.root().join("Script.c").to_string_lossy().into_owned());
    let description = resource.description().map(str::to_owned);
    let ResourceDefinitionData {
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
        rank_names,
        rank_base,
        rank_symbol_count,
        clonk_names,
    } = resource;
    let actions = action_map.map(|map| convert_action_map(&map));
    let full_core = core.clone();

    ScenarioDefinition {
        id: core.id,
        name: core.name,
        description,
        clonk_names,
        script: script.combined().to_string(),
        script_name,
        actions,
        crew_member: core.crew_member != 0,
        can_be_base: core.can_be_base,
        movement: MovementProfile::default(),
        category: core.category,
        value: core.value,
        mass: core.mass,
        picture: core.picture.map(DefinitionPicture::from),
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
        rank_names,
        rank_base,
        rank_symbol_count,
        resource_group: source_group,
        components: core
            .components
            .into_iter()
            .map(|component| DefinitionComponent {
                id: component.id,
                count: component.count,
            })
            .collect(),
        line_connect: core.line_connect,
        vertices: core.vertices,
        shape: core.shape,
        core: Some(full_core),
    }
}

pub(in crate::scenario) fn convert_action_map(map: &ResourceActionMap) -> DefinitionActions {
    let mut specs = HashMap::new();
    let mut physical = Vec::with_capacity(map.actions.len());
    let mut graphics = HashMap::new();
    graphics.insert(
        crate::PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
        DefinitionActionGraphics::default(),
    );
    let mut reflections = HashMap::new();
    for (index, (name, definition)) in map.actions.iter().enumerate() {
        let (spec, visuals) = convert_action_definition(definition);
        physical.push((name.clone(), spec.clone()));
        // SetActionByName and FnGetActMapVal both scan the physical ActMap
        // forward, so the first duplicate name wins.
        specs.entry(name.clone()).or_insert(spec);
        graphics
            .entry(name.clone())
            .or_insert_with(|| visuals.clone());
        graphics.insert(
            crate::physical_action_graphics_key(index.min(u32::MAX as usize) as u32),
            visuals,
        );
        reflections
            .entry(name.clone())
            .or_insert_with(|| crate::action::C4ActionReflection::from_resource(name, definition));
    }
    DefinitionActions {
        default_action: map.default_action.clone(),
        specs,
        physical,
        graphics,
        reflections,
    }
}

pub(crate) fn convert_action_definition(
    action: &ResourceActionDefinition,
) -> (ActionSpec, DefinitionActionGraphics) {
    let mut spec = ActionSpec::default();
    if let Some(length) = action.length {
        spec = spec.with_length(length);
    }
    if let Some(next) = &action.next_action {
        spec = spec.with_next(next.clone());
    }
    spec = spec.with_next_index(action.next_action_index);
    if let Some(procedure) = action.procedure.as_deref().and_then(|procedure| {
        clonk_resources::definition::PROCEDURE_NAMES
            .iter()
            .find(|candidate| **candidate == procedure)
    }) {
        spec = spec.with_procedure(*procedure);
    }
    if let Some(delay) = action.delay {
        spec = spec.with_delay(delay);
    }
    if let Some(step) = action.step {
        spec = spec.with_step(step);
    }
    if let Some(phase_call) = &action.phase_call {
        spec = spec.with_phase_call(phase_call.clone());
    }
    if let Some(start_call) = &action.start_call {
        spec = spec.with_start_call(start_call.clone());
    }
    if let Some(end_call) = &action.end_call {
        spec = spec.with_end_call(end_call.clone());
    }
    if let Some(abort_call) = &action.abort_call {
        spec = spec.with_abort_call(abort_call.clone());
    }
    if action.no_other_action {
        spec = spec.with_no_other_action(true);
    }
    if action.disabled {
        spec = spec.with_disabled(true);
    }
    if action.energy_usage != 0 {
        spec = spec.with_energy_usage(action.energy_usage);
    }
    if let Some(in_liquid_action) = &action.in_liquid_action {
        spec = spec.with_in_liquid_action(in_liquid_action.clone());
    }
    if let Some(directions) = action.directions {
        spec = spec.with_directions(directions);
    }
    if let Some(turn_action) = &action.turn_action {
        spec = spec.with_turn_action(turn_action.clone());
    }
    if let Some(sound) = &action.sound {
        spec = spec.with_sound(sound.clone());
    }
    if let Some(dig_free) = action.dig_free {
        spec = spec.with_dig_free(dig_free);
    }
    // ActMap Attach: the ExecAction default case zeroes dirs and
    // mobilizes instead of applying gravity (C4Object.cpp:5426-5437) —
    // dropping it made every NONE-procedure aimer/rider free-fall.
    if action.attach != 0 {
        spec = spec.with_attach(action.attach);
    }
    let mut graphics = DefinitionActionGraphics::default();
    graphics.length = action.length;
    graphics.directions = action.directions.unwrap_or(1);
    graphics.flip_dir = action.flip_dir;
    graphics.reverse = action.reverse;
    graphics.facet_base = action.facet_base;
    graphics.facet_top_face = action.facet_top_face;
    graphics.facet_target_stretch = action.facet_target_stretch;
    graphics.facet = action.facet.as_ref().map(convert_action_facet);
    (spec, graphics)
}

pub(crate) fn convert_action_facet(facet: &ResourceActionFacet) -> DefinitionActionFacet {
    DefinitionActionFacet {
        x: facet.x,
        y: facet.y,
        width: facet.width,
        height: facet.height,
        target_x: facet.target_x,
        target_y: facet.target_y,
    }
}

pub(in crate::scenario) fn read_group_file_bytes(
    group: &Group,
    path: &Path,
) -> Result<Vec<u8>, ScenarioError> {
    match group.read_file(path) {
        Ok(bytes) => Ok(bytes),
        Err(GroupError::EntryNotFound(_)) => read_file_from_fs(group, path),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            read_file_from_fs(group, path)
        }
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

fn read_file_from_fs(group: &Group, path: &Path) -> Result<Vec<u8>, ScenarioError> {
    let fallback = group.root().join(path);
    fs::read(&fallback).map_err(|_| ScenarioError::MissingScript {
        path: PathBuf::from(path),
    })
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ScenarioManifest {
    #[serde(default)]
    pub(in crate::scenario) name: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) description: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) ticks: Option<u32>,
    #[serde(default)]
    pub(in crate::scenario) ground_height: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) definitions: Vec<DefinitionManifest>,
    #[serde(default)]
    pub(in crate::scenario) initial_objects: Vec<ObjectManifest>,
    #[serde(default)]
    pub(in crate::scenario) landscape: Option<LandscapeManifest>,
    #[serde(default)]
    pub(in crate::scenario) physics: Option<PhysicsManifest>,
    #[serde(default)]
    pub(in crate::scenario) environment: Option<EnvironmentManifest>,
    #[serde(default)]
    pub(in crate::scenario) sky: Option<SkyManifest>,
    #[serde(default)]
    pub(in crate::scenario) script: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct DefinitionManifest {
    pub(in crate::scenario) id: String,
    #[serde(default)]
    pub(in crate::scenario) name: Option<String>,
    pub(in crate::scenario) script: String,
    #[serde(default)]
    pub(in crate::scenario) default_action: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) actions: HashMap<String, ActionSpec>,
    #[serde(default)]
    pub(in crate::scenario) crew_member: bool,
    #[serde(default)]
    pub(in crate::scenario) movement: Option<MovementManifest>,
    #[serde(default)]
    pub(in crate::scenario) category: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
pub(in crate::scenario) struct MovementManifest {
    #[serde(default)]
    float: Option<FloatMovementManifest>,
    #[serde(default)]
    swim: Option<SwimMovementManifest>,
    #[serde(default)]
    walk: Option<WalkMovementManifest>,
    #[serde(default)]
    scale: Option<ScaleMovementManifest>,
    #[serde(default)]
    hangle: Option<HangleMovementManifest>,
    #[serde(default)]
    dig: Option<DigMovementManifest>,
}

#[derive(Debug, Deserialize, Default)]
struct FloatMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct SwimMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct WalkMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct ScaleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct HangleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct DigMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
}

impl MovementManifest {
    pub(in crate::scenario) fn into_profile(
        self,
        id: &str,
    ) -> Result<MovementProfile, ScenarioError> {
        let mut profile = MovementProfile::default();
        if let Some(float) = self.float {
            if let Some(speed) = float.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.float_speed = speed;
            }
            if let Some(acceleration) = float.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.float_acceleration = acceleration;
            }
        }
        if let Some(swim) = self.swim {
            if let Some(speed) = swim.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.swim_speed = speed;
            }
            if let Some(acceleration) = swim.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.swim_acceleration = acceleration;
            }
        }
        if let Some(walk) = self.walk {
            if let Some(speed) = walk.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.walk_speed = speed;
            }
            if let Some(acceleration) = walk.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.walk_acceleration = acceleration;
            }
        }
        if let Some(scale) = self.scale {
            if let Some(speed) = scale.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.scale_speed = speed;
            }
            if let Some(acceleration) = scale.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.scale_acceleration = acceleration;
            }
        }
        if let Some(hangle) = self.hangle {
            if let Some(speed) = hangle.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.hangle_speed = speed;
            }
            if let Some(acceleration) = hangle.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.hangle_acceleration = acceleration;
            }
        }
        if let Some(dig) = self.dig {
            if let Some(speed) = dig.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("dig.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.dig_speed = speed;
            }
        }
        Ok(profile)
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ObjectManifest {
    pub(in crate::scenario) definition: String,
    #[serde(default)]
    pub(in crate::scenario) position: Option<[i32; 2]>,
    #[serde(default)]
    pub(in crate::scenario) velocity: Option<[i32; 2]>,
    #[serde(default)]
    pub(in crate::scenario) energy: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) owner: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) action: Option<ActionManifest>,
    #[serde(default)]
    pub(in crate::scenario) effects: Vec<EffectManifest>,
    #[serde(default)]
    pub(in crate::scenario) crew_member: Option<bool>,
    #[serde(default)]
    pub(in crate::scenario) alive: Option<bool>,
    #[serde(default)]
    pub(in crate::scenario) status: Option<ObjectStatusSpec>,
    #[serde(default)]
    pub(in crate::scenario) handle: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) container: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) category: Option<i32>,
}

#[derive(Debug)]
pub(in crate::scenario) struct ObjectStatusSpec(ObjectStatus);

impl ObjectStatusSpec {
    fn from_name(name: &str) -> Option<ObjectStatus> {
        if name.eq_ignore_ascii_case("deleted") {
            Some(ObjectStatus::Deleted)
        } else if name.eq_ignore_ascii_case("normal") {
            Some(ObjectStatus::Normal)
        } else if name.eq_ignore_ascii_case("inactive") {
            Some(ObjectStatus::Inactive)
        } else {
            None
        }
    }

    fn from_code(code: i64) -> Option<ObjectStatus> {
        match code {
            0 => Some(ObjectStatus::Deleted),
            1 => Some(ObjectStatus::Normal),
            2 => Some(ObjectStatus::Inactive),
            _ => None,
        }
    }
}

impl From<ObjectStatusSpec> for ObjectStatus {
    fn from(spec: ObjectStatusSpec) -> Self {
        spec.0
    }
}

impl<'de> Deserialize<'de> for ObjectStatusSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StatusVisitor;

        impl<'de> Visitor<'de> for StatusVisitor {
            type Value = ObjectStatusSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "an object status (\"deleted\", \"normal\", \"inactive\") or numeric code 0/1/2",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_name(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unknown object status `{value}`")))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_code(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unsupported object status code {value}")))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > i64::MAX as u64 {
                    return Err(E::custom(format!("unsupported object status code {value}")));
                }
                self.visit_i64(value as i64)
            }
        }

        deserializer.deserialize_any(StatusVisitor)
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ActionManifest {
    name: String,
    #[serde(default)]
    phase: Option<i32>,
    #[serde(default)]
    ticks: Option<i32>,
    #[serde(default)]
    data: Option<i32>,
}

impl ActionManifest {
    pub(in crate::scenario) fn into_state(self) -> ActionState {
        let mut state = ActionState::new(self.name);
        if let Some(phase) = self.phase {
            state.phase = phase;
        }
        if let Some(ticks) = self.ticks {
            state.ticks = ticks;
        }
        if let Some(data) = self.data {
            state.data = data;
        }
        state
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct EffectManifest {
    name: String,
    #[serde(default = "EffectManifest::default_priority")]
    priority: i32,
    #[serde(default = "EffectManifest::default_interval")]
    interval: i32,
    #[serde(default)]
    timer: i32,
}

impl EffectManifest {
    fn default_priority() -> i32 {
        100
    }

    fn default_interval() -> i32 {
        1
    }

    pub(in crate::scenario) fn into_state(self) -> EffectState {
        EffectState::new(self.name)
            .with_priority(self.priority)
            .with_interval(self.interval)
            .with_timer(self.timer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::scenario) enum LandscapeManifest {
    Flat { width: u32, height: i32 },
    HeightMap { width: u32, heights: Vec<i32> },
}

impl LandscapeManifest {
    pub(in crate::scenario) fn into_landscape(self) -> Result<Landscape, ScenarioError> {
        match self {
            LandscapeManifest::Flat { width, height } => Ok(Landscape::flat(width, height)),
            LandscapeManifest::HeightMap { width, heights } => Landscape::new(width, heights)
                .map_err(|error| ScenarioError::InvalidLandscape(error.to_string())),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct PhysicsManifest {
    #[serde(default)]
    pub(in crate::scenario) gravity: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_fall_speed: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_rise_speed: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_horizontal_speed: Option<i32>,
}

impl PhysicsManifest {
    pub(in crate::scenario) fn into_settings(self) -> Result<PhysicsSettings, ScenarioError> {
        let defaults = PhysicsSettings::default();
        let gravity = self.gravity.unwrap_or(defaults.gravity);
        let max_fall_speed = self.max_fall_speed.unwrap_or(defaults.max_fall_speed);
        let max_rise_speed = self.max_rise_speed.unwrap_or(defaults.max_rise_speed);

        let settings = PhysicsSettings::checked(gravity, max_fall_speed, max_rise_speed)
            .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()))?;

        if let Some(max_horizontal_speed) = self.max_horizontal_speed {
            return settings
                .with_max_horizontal_speed(max_horizontal_speed)
                .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()));
        }

        Ok(settings)
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct EnvironmentManifest {
    #[serde(default)]
    wind: Option<i32>,
    #[serde(default)]
    wind_variation: Option<i32>,
    #[serde(default)]
    wind_period: Option<u32>,
    #[serde(default)]
    temperature: Option<i32>,
    #[serde(default)]
    climate: Option<i32>,
    #[serde(default)]
    temperature_variation: Option<i32>,
    #[serde(default)]
    temperature_period: Option<u32>,
    #[serde(default)]
    temperature_phase: Option<u32>,
    #[serde(default)]
    time_of_day: Option<i32>,
    #[serde(default)]
    time_speed: Option<i32>,
    #[serde(default)]
    precipitation: Option<i32>,
    #[serde(default)]
    sky_color: Option<ColorSpec>,
    #[serde(default)]
    season: Option<i32>,
    #[serde(default)]
    year_speed: Option<i32>,
    #[serde(default)]
    temperature_range: Option<i32>,
    #[serde(default)]
    lightning: Option<i32>,
    #[serde(default)]
    meteorite: Option<i32>,
    #[serde(default)]
    volcano: Option<i32>,
    #[serde(default)]
    earthquake: Option<i32>,
    #[serde(default)]
    precipitation_strength: Option<i32>,
    #[serde(default)]
    gamma_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct SkyManifest {
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    fade_top: Option<ColorSpec>,
    #[serde(default)]
    fade_bottom: Option<ColorSpec>,
    #[serde(default)]
    scroll_mode: Option<String>,
    #[serde(default)]
    parallax_x: Option<i32>,
    #[serde(default)]
    parallax_y: Option<i32>,
    #[serde(default)]
    xdir: Option<f32>,
    #[serde(default)]
    ydir: Option<f32>,
    #[serde(default)]
    modulation: Option<ColorSpec>,
    #[serde(default)]
    back_color: Option<ColorSpec>,
}

#[derive(Debug)]
struct ColorSpec(RgbColor);

impl ColorSpec {
    fn into_color(self) -> RgbColor {
        self.0
    }
}

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = ColorSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a hex string #RRGGBB or an array [r, g, b]")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut components = Vec::with_capacity(3);
                while let Some(value) = seq.next_element::<i32>()? {
                    if !(0..=255).contains(&value) {
                        return Err(A::Error::custom(format!(
                            "color components must be between 0 and 255 (got {value})"
                        )));
                    }
                    components.push(value as u8);
                }

                if components.len() != 3 {
                    return Err(A::Error::invalid_length(
                        components.len(),
                        &"array with exactly three entries [r, g, b]",
                    ));
                }

                Ok(ColorSpec(RgbColor::new(
                    components[0],
                    components[1],
                    components[2],
                )))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_hex_color(value).map(ColorSpec).map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        fn parse_hex_color(value: &str) -> Result<RgbColor, String> {
            let trimmed = value.trim();
            let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "expected hex color in RRGGBB format, got `{}`",
                    value
                ));
            }

            let parse_component = |segment: &str| -> Result<u8, String> {
                u8::from_str_radix(segment, 16)
                    .map_err(|_| format!("invalid hex component `{segment}`"))
            };

            let r = parse_component(&hex[0..2])?;
            let g = parse_component(&hex[2..4])?;
            let b = parse_component(&hex[4..6])?;
            Ok(RgbColor::new(r, g, b))
        }

        deserializer.deserialize_any(ColorVisitor)
    }
}

impl EnvironmentManifest {
    pub(in crate::scenario) fn into_settings(self) -> EnvironmentSettings {
        let mut settings = EnvironmentSettings::new(self.wind.unwrap_or(0));
        if let Some(variation) = self.wind_variation {
            let period = self.wind_period.unwrap_or(120);
            settings = settings.with_wind_variation(variation, period);
        }
        if let Some(climate) = self.climate {
            settings = settings.with_climate(climate);
        }
        if let Some(temperature) = self.temperature {
            settings = settings.with_temperature(temperature);
        }
        if self.temperature_variation.is_some()
            || self.temperature_period.is_some()
            || self.temperature_phase.is_some()
        {
            let variation = self.temperature_variation.unwrap_or(0);
            let period = self.temperature_period.unwrap_or(600);
            let phase = self.temperature_phase.unwrap_or(0);
            settings = settings.with_temperature_cycle(variation, period, phase);
        }
        if let Some(time_of_day) = self.time_of_day {
            settings = settings.with_time_of_day(time_of_day);
        }
        if let Some(time_speed) = self.time_speed {
            settings = settings.with_time_speed(time_speed);
        }
        if let Some(precipitation) = self.precipitation {
            settings = settings.with_precipitation(precipitation);
            if self.precipitation_strength.is_none() {
                settings = settings.with_precipitation_strength(precipitation);
            }
        }
        if let Some(color) = self.sky_color {
            settings = settings.with_sky_color(color.into_color());
        }
        if let Some(season) = self.season {
            settings = settings.with_season(season);
        }
        if let Some(year_speed) = self.year_speed {
            settings = settings.with_year_speed(year_speed);
        }
        if let Some(range) = self.temperature_range {
            settings = settings.with_temperature_range(range);
        }
        if let Some(lightning) = self.lightning {
            settings = settings.with_lightning(lightning);
        }
        if let Some(meteorite) = self.meteorite {
            settings = settings.with_meteorite(meteorite);
        }
        if let Some(volcano) = self.volcano {
            settings = settings.with_volcano(volcano);
        }
        if let Some(earthquake) = self.earthquake {
            settings = settings.with_earthquake(earthquake);
        }
        if let Some(strength) = self.precipitation_strength {
            settings = settings.with_precipitation_strength(strength);
        }
        if let Some(enabled) = self.gamma_enabled {
            settings = if enabled {
                settings.with_gamma_enabled()
            } else {
                settings.with_gamma_disabled()
            };
        }
        settings
    }
}

impl SkyManifest {
    pub(in crate::scenario) fn into_config(
        self,
        group: &Group,
    ) -> Result<SkyConfig, ScenarioError> {
        let mut settings = SkySettings::default();
        let mut surface_image = None;

        if let Some(surface_name) = self.surface {
            let path = PathBuf::from(&surface_name);
            let bytes = match group.read_file(&path) {
                Ok(bytes) => bytes,
                Err(GroupError::EntryNotFound(_)) => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(error) => return Err(ScenarioError::Resources(error)),
            };

            let decoded =
                load_from_memory(&bytes).map_err(|source| ScenarioError::SkySurfaceDecode {
                    path: path.clone(),
                    source,
                })?;
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();
            settings = settings.with_surface(width, height);
            surface_image = Some(Arc::new(GraphicsImage::new(width, height, pixels)));
        }

        if let Some(color) = self.fade_top {
            settings.fade_top = color.into_color();
        }
        if let Some(color) = self.fade_bottom {
            settings.fade_bottom = color.into_color();
        }
        if let Some(mode) = self.scroll_mode {
            settings.parallax_mode = parse_scroll_mode(&mode)?;
        }
        if let Some(value) = self.parallax_x {
            settings.parallax_x = value;
        }
        if let Some(value) = self.parallax_y {
            settings.parallax_y = value;
        }
        if let Some(value) = self.xdir {
            settings.base_xdir = value;
        }
        if let Some(value) = self.ydir {
            settings.base_ydir = value;
        }
        if let Some(color) = self.modulation {
            settings.modulation = Some(rgb_to_bgr_u32(color.into_color()));
        }
        if let Some(color) = self.back_color {
            let back_color = rgb_to_bgr_u32(color.into_color());
            settings.back_color = Some(back_color);
            settings.back_color_raw = back_color;
        }

        Ok(SkyConfig {
            settings,
            surface: surface_image,
        })
    }
}

fn parse_scroll_mode(value: &str) -> Result<SkyParallaxMode, ScenarioError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(SkyParallaxMode::Fixed);
    }
    if let Ok(code) = trimmed.parse::<i32>() {
        return match code {
            0 => Ok(SkyParallaxMode::Fixed),
            1 => Ok(SkyParallaxMode::Wind),
            2 => Ok(SkyParallaxMode::Parallax),
            other => Err(ScenarioError::InvalidSky(format!(
                "unknown sky scroll mode code {other}"
            ))),
        };
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "fixed" => Ok(SkyParallaxMode::Fixed),
        "wind" => Ok(SkyParallaxMode::Wind),
        "parallax" => Ok(SkyParallaxMode::Parallax),
        other => Err(ScenarioError::InvalidSky(format!(
            "unknown sky scroll mode `{other}`"
        ))),
    }
}

fn rgb_to_bgr_u32(color: RgbColor) -> u32 {
    u32::from(color.b) | (u32::from(color.g) << 8) | (u32::from(color.r) << 16)
}
