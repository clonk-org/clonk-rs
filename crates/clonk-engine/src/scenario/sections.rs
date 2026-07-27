//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::scenario) fn load_legacy_scenario_sections(
    group: &Group,
    main_manifest: &LegacyScenarioManifest,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    root_section_name: &str,
    main_landscape: &Option<Landscape>,
    main_landscape_systems: &ScenarioLandscapeSystems,
    main_objects: &[ScenarioSpawn],
    main_environment: EnvironmentSettings,
    has_sky_surface: bool,
    map_callback_functions: &HashSet<String>,
    main_post_init_map_callbacks: &crate::map_creator_s2::PostInitMapCallbacks,
) -> Result<Vec<ScenarioSectionSpec>, ScenarioError> {
    let classifier_baseline = classifier.as_deref().cloned();
    let persistent_runtime = main_landscape.as_ref().map(|landscape| LandscapeGameData {
        map_seed: landscape.map_seed(),
        mat_modulation: landscape.modulation(),
        ..LandscapeGameData::default()
    });
    let main_s2_source = try_read_group_file_case_insensitive(group, "Landscape.txt")?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let main_s2_diff = try_read_group_file_case_insensitive(group, "DiffLandscape.bmp")
        .ok()
        .flatten()
        .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok());
    let main_has_s2_creator = main_landscape
        .as_ref()
        .and_then(Landscape::raster_state)
        .and_then(LandscapeRasterState::map_creator)
        .is_some();
    let main_s2_overload = main_s2_source
        .filter(|_| main_has_s2_creator)
        .map(|source| ScenarioSectionS2Spec {
            source,
            map_width: main_manifest.core.landscape.map_width,
            map_height: main_manifest.core.landscape.map_height,
            map_player_extend: main_manifest.core.landscape.map_player_extend,
            player_count: startup_player_count,
            map_zoom: main_manifest.core.landscape.map_zoom,
            diff: main_s2_diff,
            left_open: main_manifest.core.landscape.left_open,
            right_open: main_manifest.core.landscape.right_open,
            top_open: main_manifest.core.landscape.top_open,
            bottom_open: main_manifest.core.landscape.bottom_open,
            auto_scan_side_open: main_manifest.core.landscape.auto_scan_side_open,
            no_scan: main_manifest.core.landscape.no_scan,
            shade_materials: main_manifest.core.landscape.shade_materials,
            script_functions: map_callback_functions.clone(),
        });
    let mut sections = vec![ScenarioSectionSpec {
        // An exact save stores the live current section in the root and
        // identifies it through Game.CurrentScenarioSection. `SectMain.c4g`
        // is then a distinct departed section when the current one is not
        // Main (C4GameSave::SaveScenarioSections).
        name: root_section_name.to_string(),
        source_group: Some(group.clone()),
        landscape: main_landscape.clone(),
        landscape_systems: main_landscape_systems.clone(),
        exact_landscape: main_manifest.core.landscape.exact_landscape,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: main_landscape
            .as_ref()
            .and_then(Landscape::raster_state)
            .and_then(LandscapeRasterState::map_creator)
            .cloned(),
        s2_overload: main_s2_overload,
        gravity: main_manifest.core.landscape.gravity,
        post_init_map_callbacks: main_post_init_map_callbacks.clone(),
        keep_map_creator: main_manifest.core.landscape.keep_map_creator,
        no_initialize: main_manifest.core.head.no_initialize != 0,
        objects: main_objects.to_vec(),
        scenario_values: ScenarioValueStore::from_runtime_core(
            &main_manifest.core,
            has_sky_surface,
        )
        .with_section_head_defaults(&main_manifest.core.head),
        base_reject_entrance_enabled: (main_manifest.core.game.realism.base_functionality
            & BASEFUNC_REJECT_ENTRANCE)
            != 0,
        base_extinguish_enabled: (main_manifest.core.game.realism.base_functionality
            & BASEFUNC_EXTINGUISH)
            != 0,
        environment: main_environment,
    }];

    let mut discovered = Vec::new();
    for entry in group.entries()? {
        let Some(name) = legacy_scenario_section_name(&entry.relative_path)? else {
            continue;
        };
        // The root always wins a stale duplicate. Unlike the old hard-coded
        // Main filter, this retains SectMain when another section is current.
        if !name.eq_ignore_ascii_case(root_section_name) {
            discovered.push((name, entry.relative_path));
        }
    }
    discovered.sort_by(|(left, _), (right, _)| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });

    for (name, path) in discovered {
        let mut section_classifier = classifier_baseline.clone();
        if let Some(classifier) = section_classifier.as_mut() {
            classifier.clear_texmap_lookups();
        }
        let section_group = group.open_child(path)?;
        let manifest = match parse_legacy_scenario_manifest(&section_group) {
            Ok(overlay) => Some(overlay_legacy_scenario_manifest(main_manifest, overlay)?),
            Err(ScenarioError::LegacyCoreMissing) => None,
            Err(error) => return Err(error),
        };
        let manifest = manifest.as_ref().unwrap_or(main_manifest);
        let s2_source = try_read_group_file_case_insensitive(&section_group, "Landscape.txt")?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let s2_diff = try_read_group_file_case_insensitive(&section_group, "DiffLandscape.bmp")
            .ok()
            .flatten()
            .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok());
        let mut post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut prepared_map_creator = None;
        let mut landscape = load_legacy_landscape(
            &section_group,
            manifest,
            persistent_runtime.as_ref(),
            true,
            section_classifier.as_mut(),
            random_seed,
            startup_player_count,
            map_callback_functions,
            &mut post_init_map_callbacks,
            &mut prepared_map_creator,
        )?;
        let landscape_systems = load_legacy_landscape_systems(&section_group)?;
        if let (Some(runtime), Some(landscape)) = (persistent_runtime, landscape.as_mut()) {
            landscape.set_modulation(runtime.mat_modulation);
        }
        let texmap_lookups = section_classifier
            .as_ref()
            .map(|classifier| classifier.texmap_lookups().to_vec())
            .unwrap_or_default();
        let resynthesize_static_map = !manifest.core.landscape.exact_landscape
            && landscape
                .as_ref()
                .and_then(Landscape::raster_state)
                .is_some_and(|state| state.map().is_some() && state.map_creator().is_none());
        let environment = derive_legacy_environment(manifest)?;
        let scenario_values =
            ScenarioValueStore::from_runtime_core(&manifest.core, has_sky_surface)
                .with_section_head_defaults(&main_manifest.core.head);
        let has_s2_overload = prepared_map_creator.is_some() && s2_source.is_some();
        sections.push(ScenarioSectionSpec {
            name,
            source_group: Some(section_group),
            landscape,
            landscape_systems,
            exact_landscape: manifest.core.landscape.exact_landscape,
            texmap_lookups,
            resynthesize_static_map,
            map_creator: prepared_map_creator,
            s2_overload: has_s2_overload
                .then(|| {
                    s2_source.map(|source| ScenarioSectionS2Spec {
                        source,
                        map_width: manifest.core.landscape.map_width,
                        map_height: manifest.core.landscape.map_height,
                        map_player_extend: manifest.core.landscape.map_player_extend,
                        player_count: startup_player_count,
                        map_zoom: manifest.core.landscape.map_zoom,
                        diff: s2_diff,
                        left_open: manifest.core.landscape.left_open,
                        right_open: manifest.core.landscape.right_open,
                        top_open: manifest.core.landscape.top_open,
                        bottom_open: manifest.core.landscape.bottom_open,
                        auto_scan_side_open: manifest.core.landscape.auto_scan_side_open,
                        no_scan: manifest.core.landscape.no_scan,
                        shade_materials: manifest.core.landscape.shade_materials,
                        script_functions: map_callback_functions.clone(),
                    })
                })
                .flatten(),
            gravity: manifest.core.landscape.gravity,
            post_init_map_callbacks,
            keep_map_creator: manifest.core.landscape.keep_map_creator,
            no_initialize: manifest.core.head.no_initialize != 0,
            // C4ScenarioSection retains the child group but does not compile
            // Objects.txt during scenario discovery. C4GameObjects::Load
            // reopens and compiles it on every activation against the then-
            // current process-global C4StringTable.
            objects: Vec::new(),
            scenario_values,
            base_reject_entrance_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_REJECT_ENTRANCE)
                != 0,
            base_extinguish_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_EXTINGUISH)
                != 0,
            environment,
        });
    }

    Ok(sections)
}

pub(in crate::scenario) fn collect_legacy_objects(
    group: &Group,
    definitions: &[ScenarioDefinition],
    string_registrations: &clonk_script::StringRegistrations,
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let definition_ids = definitions
        .iter()
        .map(|definition| definition.id.as_str())
        .collect::<HashSet<_>>();
    collect_legacy_objects_with_definition_ids(
        group,
        &definition_ids,
        string_registrations,
        &HashSet::new(),
    )
}

/// Compile one section's Objects.txt at its C4GameObjects::Load boundary.
/// Section groups do not own a string table: S# values resolve against the
/// process-global table as it exists at this activation.
pub(crate) fn collect_legacy_objects_with_definition_ids(
    group: &Group,
    definition_ids: &HashSet<&str>,
    string_registrations: &clonk_script::StringRegistrations,
    retained_object_numbers: &HashSet<u64>,
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let bytes = match group.read_file("Objects.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(Vec::new()),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // C++ reads Objects.txt as raw bytes in the config charset; fall back to
    // a Latin-1 decode so Windows-1252 umlauts survive (Drachenfels.c4s).
    let text = String::from_utf8(bytes).unwrap_or_else(|error| {
        error
            .into_bytes()
            .iter()
            .map(|&byte| byte as char)
            .collect()
    });
    let mut records = parse_legacy_objects(&text)?;
    if records.is_empty() {
        return Ok(Vec::new());
    }

    let mut index_by_number: HashMap<u64, usize> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        if let Some(number) = record.number {
            index_by_number.insert(number, index);
        }
    }

    for index in 0..records.len() {
        let parent_number = match records[index].number {
            Some(value) => value,
            None => continue,
        };
        let child_numbers: Vec<u64> = records[index].contents.clone();
        for child_number in child_numbers {
            if child_number == 0 || child_number == parent_number {
                continue;
            }
            if let Some(child_index) = index_by_number.get(&child_number).copied() {
                if records[child_index].contained.is_none()
                    && records[child_index].inferred_container.is_none()
                {
                    records[child_index].inferred_container = Some(parent_number);
                }
            }
        }
    }

    // C4GameObjects::ObjectPointer searches both the newly compiled main
    // list and the retained inactive list. Section loads therefore resolve
    // saved command pointers to preserved objects as well as sibling rows.
    let mut object_numbers = retained_object_numbers.clone();
    object_numbers.extend(
        records
            .iter()
            .filter(|record| !matches!(record.status, Some(ObjectStatus::Deleted)))
            .filter(|record| {
                record
                    .id
                    .as_deref()
                    .is_some_and(|id| definition_ids.contains(id))
            })
            .filter_map(|record| record.number),
    );
    let value_resolution = SerializedC4ValueResolution {
        object_numbers: &object_numbers,
        string_registrations,
    };

    let mut spawns = Vec::new();
    for record in records.into_iter() {
        if let Some(spawn) = record.into_spawn(definition_ids, &value_resolution)? {
            spawns.push(spawn);
        }
    }
    Ok(spawns)
}

/// C4StringTable::Load assigns each Strings.txt line its zero-based enum ID.
/// Repeated text reuses the existing C4String and updates that one instance to
/// the later ID, so the earlier ID is no longer resolvable
/// (C4StringTable.cpp:201-216).
pub(in crate::scenario) fn load_legacy_string_table(
    group: &Group,
) -> Result<clonk_script::StringRegistrations, ScenarioError> {
    let string_registrations = clonk_script::new_string_registrations();
    let bytes = match group.read_file("Strings.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(string_registrations),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(string_registrations);
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // SCopySegment/SCharPos scan the component as a C string. Bytes after
    // the first embedded NUL are therefore invisible to the whole line walk.
    let bytes = &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())];
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let index = i32::try_from(index).unwrap_or(i32::MAX);
        // SCopySegment copies at most C4AUL_MAX_String bytes and
        // SReplaceChar turns the first CR into the string terminator
        // (C4StringTable.cpp:208-211).
        let end = line
            .iter()
            .position(|byte| *byte == b'\r')
            .unwrap_or(line.len())
            .min(1024);
        // C4StringTable::Load passes the component bytes straight to
        // RegString. Strings.txt is not presentation text and must not pass
        // through the CP1252-to-Unicode decoder used by names/descriptions.
        let value = clonk_script::c4_string_from_bytes(&line[..end]);
        // RegisterLoaded performs the native C-string-prefix lookup. Equal
        // lines reuse one C4String identity and the later line overwrites
        // that shared instance's current enumeration ID.
        clonk_script::register_loaded_c4_string(&string_registrations, index, &value);
    }
    Ok(string_registrations)
}

#[derive(Debug, Default)]
pub(in crate::scenario) struct LegacyObjectRecord {
    line: usize,
    id: Option<String>,
    pub(in crate::scenario) number: Option<u64>,
    /// C4Object::CustomName (`Name=`, C4Object.cpp:2749-2760).
    custom_name: Option<String>,
    /// Player-owned C4ObjectInfo lookup name (`Info=`).
    pub(in crate::scenario) info_name: Option<String>,
    status: Option<ObjectStatus>,
    pub(in crate::scenario) owner: Option<i32>,
    /// C4Object::Controller, compiled verbatim with default NO_OWNER
    /// (C4Object.cpp:2739).
    controller: Option<i32>,
    /// Kill attribution cached by C4Object (`LastEngLossPlr=`).
    last_energy_loss_cause: Option<i32>,
    pub(in crate::scenario) x: Option<i32>,
    pub(in crate::scenario) y: Option<i32>,
    motion_x: Option<i32>,
    motion_y: Option<i32>,
    /// The frame of the most recent solid-attachment movement. Native uses
    /// -1 as its compile default; retain the signed word until spawn wiring
    /// converts it to the engine's optional frame representation.
    last_attach_movement_frame: Option<i32>,
    no_collect_delay: Option<i32>,
    base: Option<i32>,
    /// Saved velocity is C4Fixed (C4Object.cpp:2765-2766), float-encoded
    /// in real content (`XDir=f...`).
    pub(in crate::scenario) xdir: Option<crate::math::C4Fixed>,
    pub(in crate::scenario) ydir: Option<crate::math::C4Fixed>,
    /// Saved sub-pixel position/rotation and angular velocity — the same
    /// C4Fixed encoding (FixX/FixY/FixR/RDir, C4Object.cpp:2762-2767).
    /// C++ reads them INDEPENDENTLY of the integer X/Y/Rotation and never
    /// reconciles after load.
    pub(in crate::scenario) fix_x: Option<crate::math::C4Fixed>,
    pub(in crate::scenario) fix_y: Option<crate::math::C4Fixed>,
    pub(in crate::scenario) fix_r: Option<crate::math::C4Fixed>,
    pub(in crate::scenario) rdir: Option<crate::math::C4Fixed>,
    /// C4Object::Mobile, serialized with default false (C4Object.cpp:2772).
    pub(in crate::scenario) mobile: Option<bool>,
    solid_mask: Option<Vec<i32>>,
    /// Whole-degree rotation (`Rotation=`, C4Object.cpp:2744).
    pub(in crate::scenario) rotation: Option<i32>,
    /// Mid-cycle Def TimerCall counter (`Timer=`, default 0,
    /// C4Object.cpp:2738).
    timer: Option<i32>,
    /// Numbered C4Object::Local slots (`Locals=`, C4Object.cpp:2788;
    /// C4ValueList::CompileFunc, C4ValueList.cpp:102-136).
    locals: Option<Vec<SerializedC4Value>>,
    /// Per-object script locals (`LocalNamed=`, C4Object.cpp:2788;
    /// C4ValueMapData::CompileFunc, C4ValueMap.cpp:236-295).
    local_named: Option<Vec<(String, SerializedC4Value)>>,
    /// The CURRENT shape's vertices, serialized by C4Shape::CompileFunc
    /// into the [Object] section (C4Shape.cpp:495-515): the effective
    /// post-Con/rotation shape, loaded verbatim.
    vertex_count: Option<i32>,
    vertex_x: Option<Vec<i32>>,
    vertex_y: Option<Vec<i32>>,
    vertex_cnat: Option<Vec<i32>>,
    vertex_friction: Option<Vec<i32>>,
    /// Exact live C4Shape rectangle compiled inline into the Object section.
    shape_width: Option<i32>,
    shape_height: Option<i32>,
    shape_offset: Option<Vec<i32>>,
    /// Exact live C4Shape::FireTop; missing values compile as zero.
    shape_fire_top: Option<i32>,
    /// C4Object::fOwnVertices. The original vertex copy occupies raw shape
    /// slots 15.. and is used by later UpdateShape calls.
    own_vertices: Option<bool>,
    /// Saved live C4Shape::ContactDensity (C4Shape.cpp:495-510).
    contact_density: Option<i32>,
    shape_attach_x: Option<i32>,
    shape_attach_y: Option<i32>,
    shape_attach_vertex: Option<i32>,
    own_mass: Option<i32>,
    /// C4Object::Mass is compiled independently of OwnMass. C++ currently
    /// refreshes derived mass on later construction changes, not as part of
    /// Objects.txt parsing, so keep the serialized cache available.
    mass: Option<i32>,
    pub(in crate::scenario) damage: Option<i32>,
    pub(in crate::scenario) energy: Option<i32>,
    /// C4Object::NeedEnergy (`NeedEnergy=`, C4Object.cpp:2805).
    need_energy: Option<bool>,
    /// C4Object::Select (`Selected=`, C4Object.cpp:2800).
    selected: Option<bool>,
    /// C4Object::MagicEnergy (`MagicEnergy=`, C4Object.cpp:2768).
    magic_energy: Option<i32>,
    pub(in crate::scenario) construction: Option<i32>,
    alive: Option<bool>,
    pub(in crate::scenario) breath: Option<i32>,
    fire_phase: Option<i32>,
    on_fire: Option<bool>,
    in_liquid: Option<bool>,
    /// C4Object::EntranceStatus (`EntranceStatus=`, C4Object.cpp:2803).
    entrance_status: Option<bool>,
    physical_temporary: Option<bool>,
    pub(in crate::scenario) ocf: Option<u32>,
    category: Option<i32>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action_name: Option<String>,
    action_phase: Option<i32>,
    /// Action.Time (`ActionTime=`, C4Object.cpp:2745 area).
    action_ticks: Option<i32>,
    /// Action.PhaseDelay (`PhaseDelay=`), the intra-phase counter.
    action_phase_delay: Option<i32>,
    action_data: Option<i32>,
    action_target: Option<i32>,
    action_target2: Option<i32>,
    /// Raw C4EnumeratedObjectPtr::number for C4Object::pLayer. Keep the
    /// signed cache word even when denumeration cannot resolve a pointer.
    layer: Option<i32>,
    /// C4Object::Visibility (`Visibility=`, C4Object.cpp:2814).
    visibility: Option<i32>,
    /// C4Object::BlitMode (`BlitMode=`, C4Object.cpp:2817).
    pub(in crate::scenario) blit_mode: Option<u32>,
    /// C4Object::Color (`Color=`/`ColorDw=`, C4Object.cpp:2786-2787).
    pub(in crate::scenario) color: Option<u32>,
    /// C4Object::ColorMod (`ColorMod=`, C4Object.cpp:2816).
    pub(in crate::scenario) color_modulation: Option<u32>,
    /// C4Object::PictureRect (`Picture=`, C4Object.cpp:2798).
    picture_rect: Option<DefinitionRect>,
    plr_view_range: Option<i32>,
    crew_disabled: Option<bool>,
    base_graphics: Option<crate::ObjectBaseGraphics>,
    pub(in crate::scenario) draw_transform: Option<crate::DrawTransform>,
    pub(in crate::scenario) effects: Option<Vec<SerializedEffectState>>,
    graphics_overlays: Option<Vec<SerializedObjectGraphicsOverlay>>,
    pub(in crate::scenario) temporary_physical: Option<crate::PhysicalInfo>,
    pub(in crate::scenario) physical_changes: Vec<(String, i32)>,
    /// StdCompilerINIRead removes the first matching naming node after it is
    /// consumed, so duplicate C4PhysicalInfo names never overwrite it.
    physical_fields_seen: HashSet<String>,
    pub(in crate::scenario) commands: BTreeMap<usize, SerializedLegacyCommand>,
    /// Saved C4Object::Component (`Component=WOOD=5;METL=1;`).
    components: Option<Vec<(DefinitionId, i32)>>,
    /// Raw C4EnumeratedObjectPtr::number for C4Object::Contained.
    contained: Option<i32>,
    /// Live relationship inferred from a parent's Contents list when the
    /// child's serialized Contained cache was absent. Keep this separate so
    /// GetObjectVal still observes the native zero compiler default.
    inferred_container: Option<u64>,
    pub(in crate::scenario) contents: Vec<u64>,
}

/// Object references inside a graphics overlay are denumerated only after
/// all Objects.txt rows have been accepted. Keep the raw signed number here
/// while parsing, just like [`SerializedC4Value::ObjectNumber`].
#[derive(Debug, Clone, PartialEq)]
struct SerializedObjectGraphicsOverlay {
    id: i32,
    mode: crate::GraphicsOverlayMode,
    definition: Option<DefinitionId>,
    graphics_name: Option<String>,
    action: Option<String>,
    blit_mode: u32,
    phase: i32,
    transform: crate::DrawTransform,
    color_modulation: u32,
    overlay_object: i32,
}

/// Exact C4Command::CompileFunc projection. Integer flags intentionally remain
/// integers: the native fields are int32 words and old saves may contain
/// non-canonical truthy values. The parser also accepts the native `$1` and
/// unversioned layouts; all versions resolve into this current representation.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::scenario) struct SerializedLegacyCommand {
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) tx: SerializedC4Value,
    pub(in crate::scenario) ty: i32,
    target: i32,
    target2: i32,
    data: i32,
    update_interval: i32,
    pub(in crate::scenario) evaluated: i32,
    path_checked: i32,
    finished: i32,
    failures: i32,
    retries: i32,
    permit: i32,
    pub(in crate::scenario) base_mode: i32,
    pub(in crate::scenario) text: String,
}

fn denumerate_legacy_object_number(raw: i32, object_numbers: &HashSet<u64>) -> Option<ObjectId> {
    let number = if (1_000_000_000..=1_001_000_000).contains(&raw) {
        raw - 1_000_000_000
    } else {
        raw
    };
    u64::try_from(number)
        .ok()
        .filter(|number| *number != 0 && object_numbers.contains(number))
        .map(ObjectId::new)
}

impl SerializedObjectGraphicsOverlay {
    fn resolve(self, object_numbers: &HashSet<u64>) -> crate::ObjectGraphicsOverlay {
        crate::ObjectGraphicsOverlay {
            id: self.id,
            mode: self.mode,
            definition: self.definition,
            graphics_name: self.graphics_name,
            action: self.action,
            phase: self.phase,
            blit_mode: self.blit_mode,
            color_modulation: self.color_modulation,
            overlay_object: denumerate_legacy_object_number(self.overlay_object, object_numbers),
            transform: Some(self.transform),
        }
    }
}

impl SerializedLegacyCommand {
    fn resolve(
        self,
        _line: usize,
        resolution: &SerializedC4ValueResolution<'_>,
    ) -> Result<crate::command::LegacyCommandSave, ScenarioError> {
        let is_call = self.name == "Call";
        let tx_value = self.tx.resolve(resolution);
        let (tx, tx_definition) = match &tx_value {
            clonk_script::Value::Nil => (None, None),
            clonk_script::Value::Int(value) => (Some(*value), None),
            clonk_script::Value::C4Id(value) => (None, Some(value.clone())),
            _ => (None, None),
        };
        Ok(crate::command::LegacyCommandSave {
            view: crate::command::CommandView {
                name: self.name,
                target: denumerate_legacy_object_number(self.target, resolution.object_numbers),
                tx,
                tx_value: Some(tx_value),
                tx_definition,
                ty: Some(self.ty),
                target2: denumerate_legacy_object_number(self.target2, resolution.object_numbers),
                data: crate::command::CommandData::Integer(self.data),
                legacy_data: is_call.then_some(self.data),
                finished: self.finished != 0,
            },
            update_interval: self.update_interval,
            evaluated: self.evaluated,
            path_checked: self.path_checked,
            finished: self.finished,
            failures: self.failures,
            retries: self.retries,
            permit: self.permit,
            base_mode: self.base_mode,
            text: self.text,
        })
    }
}

impl LegacyObjectRecord {
    fn new(line: usize) -> Self {
        Self {
            line,
            ..Self::default()
        }
    }

    fn apply_property(&mut self, key: &str, value: &str) -> Result<(), ScenarioError> {
        // StdCompilerINIRead looks up naming nodes byte-for-byte. Map only
        // the exact spellings used by C4Object::CompileFunc; a wrong-case
        // line is an unused naming and leaves the compile default intact.
        let normalized_key = match key {
            "id" => "id",
            "Name" => "name",
            "Number" => "number",
            "Status" => "status",
            "Info" => "info",
            "Owner" => "owner",
            "Timer" => "timer",
            "Controller" => "controller",
            "LastEngLossPlr" => "lastenglossplr",
            "Category" => "category",
            "X" => "x",
            "Y" => "y",
            "Rotation" => "rotation",
            "MotionX" => "motionx",
            "MotionY" => "motiony",
            "LastSolidAtchFrame" => "lastsolidatchframe",
            "NoCollectDelay" => "nocollectdelay",
            "Base" => "base",
            "Size" => "size",
            "OwnMass" => "ownmass",
            "Mass" => "mass",
            "Damage" => "damage",
            "Energy" => "energy",
            "MagicEnergy" => "magicenergy",
            "Alive" => "alive",
            "Breath" => "breath",
            "FirePhase" => "firephase",
            "Color" => "color",
            "ColorDw" => "colordw",
            "Locals" => "locals",
            "FixX" => "fixx",
            "FixY" => "fixy",
            "FixR" => "fixr",
            "XDir" => "xdir",
            "YDir" => "ydir",
            "RDir" => "rdir",
            "Width" => "width",
            "Height" => "height",
            "Offset" => "offset",
            "Vertices" => "vertices",
            "VertexX" => "vertexx",
            "VertexY" => "vertexy",
            "VertexCNAT" => "vertexcnat",
            "VertexFriction" => "vertexfriction",
            "ContactDensity" => "contactdensity",
            "FireTop" => "firetop",
            "AttachX" => "attachx",
            "AttachY" => "attachy",
            "AttachVtx" => "attachvtx",
            "OwnVertices" => "ownvertices",
            "SolidMask" => "solidmask",
            "Picture" => "picture",
            "Mobile" => "mobile",
            "Selected" => "selected",
            "OnFire" => "onfire",
            "InLiquid" => "inliquid",
            "EntranceStatus" => "entrancestatus",
            "PhysicalTemporary" => "physicaltemporary",
            "NeedEnergy" => "needenergy",
            "OCF" => "ocf",
            "Action" => "action",
            "Dir" => "dir",
            "ComDir" => "comdir",
            "ActionTime" => "actiontime",
            "ActionData" => "actiondata",
            "Phase" => "phase",
            "PhaseDelay" => "phasedelay",
            "Contained" => "contained",
            "ActionTarget1" => "actiontarget1",
            "ActionTarget2" => "actiontarget2",
            "Component" => "component",
            "Contents" => "contents",
            "PlrViewRange" => "plrviewrange",
            "Visibility" => "visibility",
            "LocalNamed" => "localnamed",
            "ColorMod" => "colormod",
            "BlitMode" => "blitmode",
            "CrewDisabled" => "crewdisabled",
            "Layer" => "layer",
            "Graphics" => "graphics",
            "DrawTransform" => "drawtransform",
            "Effects" => "effects",
            "GfxOverlay" => "gfxoverlay",
            _ => return Ok(()),
        };
        let trimmed_value = value.trim();
        match normalized_key {
            "id" => {
                self.id = Some(trimmed_value.to_string());
            }
            "number" => {
                let number = parse_i64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Number `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                if number < 0 {
                    return Err(ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: Number must be >= 0 (got {})",
                        self.line, number
                    )));
                }
                self.number = Some(number as u64);
            }
            "name" => {
                self.custom_name = parse_legacy_object_name(trimmed_value, self.line)?;
            }
            "info" => {
                // nInfo is compiled through RCT_All: leading horizontal
                // whitespace is skipped, but the remainder of the physical
                // line (including `//` and trailing spaces) is data.
                let whole_line = value.trim_start_matches([' ', '\t']);
                self.info_name = (!whole_line.is_empty()).then(|| whole_line.to_string());
            }
            "status" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Status `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.status = Some(ObjectStatus::from_script_value(raw).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: unsupported Status value {}",
                        self.line, raw
                    ))
                })?);
            }
            "owner" => {
                let owner = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Owner `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.owner = Some(owner);
            }
            "controller" => {
                let controller = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Controller `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.controller = Some(controller);
            }
            "lastenglossplr" => {
                self.last_energy_loss_cause = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "LastEngLossPlr",
                )?);
            }
            "x" => {
                self.x = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid X `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "y" => {
                self.y = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Y `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "motionx" => {
                self.motion_x = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MotionX `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "motiony" => {
                self.motion_y = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MotionY `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "lastsolidatchframe" => {
                self.last_attach_movement_frame = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "LastSolidAtchFrame",
                )?);
            }
            "nocollectdelay" => {
                self.no_collect_delay = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "NoCollectDelay",
                )?);
            }
            "base" => {
                self.base = Some(parse_object_i32(trimmed_value, self.line, "Base")?);
            }
            "xdir" => {
                self.xdir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid XDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "ydir" => {
                self.ydir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid YDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixx" => {
                self.fix_x = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixX `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixy" => {
                self.fix_y = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixY `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixr" => {
                self.fix_r = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixR `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "rdir" => {
                self.rdir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid RDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "mobile" => {
                let mobile = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Mobile `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.mobile = Some(mobile);
            }
            "solidmask" => {
                // C4Object::CompileFunc SolidMask (default Def->SolidMask,
                // C4Object.cpp:2770): six ints; 0,0,0,0,0,0 = mask OFF.
                self.solid_mask = Some(parse_i32_list(trimmed_value, self.line, "SolidMask")?);
            }
            "rotation" => {
                self.rotation = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Rotation `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "timer" => {
                self.timer = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Timer `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "locals" => {
                self.locals = Some(parse_local_slots(trimmed_value, self.line)?);
            }
            "localnamed" => {
                self.local_named = Some(parse_local_named(trimmed_value, self.line)?);
            }
            "width" => {
                self.shape_width = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Width `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "height" => {
                self.shape_height = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Height `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "offset" => {
                self.shape_offset = Some(parse_i32_list(trimmed_value, self.line, "Offset")?);
            }
            "vertices" => {
                self.vertex_count = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Vertices `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "vertexx" => {
                self.vertex_x = Some(parse_i32_list(trimmed_value, self.line, "VertexX")?);
            }
            "vertexy" => {
                self.vertex_y = Some(parse_i32_list(trimmed_value, self.line, "VertexY")?);
            }
            "vertexcnat" => {
                self.vertex_cnat = Some(parse_i32_list(trimmed_value, self.line, "VertexCNAT")?);
            }
            "vertexfriction" => {
                self.vertex_friction =
                    Some(parse_i32_list(trimmed_value, self.line, "VertexFriction")?);
            }
            "ownvertices" => {
                let own_vertices = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid OwnVertices `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.own_vertices = Some(own_vertices);
            }
            "contactdensity" => {
                self.contact_density = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ContactDensity `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "firetop" => {
                self.shape_fire_top = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FireTop `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "attachx" => {
                self.shape_attach_x = Some(parse_object_i32(trimmed_value, self.line, "AttachX")?);
            }
            "attachy" => {
                self.shape_attach_y = Some(parse_object_i32(trimmed_value, self.line, "AttachY")?);
            }
            "attachvtx" => {
                self.shape_attach_vertex =
                    Some(parse_object_i32(trimmed_value, self.line, "AttachVtx")?);
            }
            "ownmass" => {
                self.own_mass = Some(parse_object_i32(trimmed_value, self.line, "OwnMass")?);
            }
            "mass" => {
                self.mass = Some(parse_object_i32(trimmed_value, self.line, "Mass")?);
            }
            "damage" => {
                self.damage = Some(parse_object_i32(trimmed_value, self.line, "Damage")?);
            }
            "energy" => {
                self.energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Energy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "breath" => {
                self.breath = Some(parse_object_i32(trimmed_value, self.line, "Breath")?);
            }
            "firephase" => {
                self.fire_phase = Some(parse_object_i32(trimmed_value, self.line, "FirePhase")?);
            }
            "needenergy" => {
                let need_energy = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid NeedEnergy `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.need_energy = Some(need_energy);
            }
            "selected" => {
                let selected = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Selected `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.selected = Some(selected);
            }
            "onfire" => {
                self.on_fire = Some(parse_object_bool(trimmed_value, self.line, "OnFire")?);
            }
            // C4Object::MagicEnergy compiles verbatim with default 0
            // (C4Object.cpp:2768) — Drachenfels' wizards carry it.
            "magicenergy" => {
                self.magic_energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MagicEnergy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            // C++ saves Con under the key "Size" (C4Object::CompileFunc,
            // C4Object.cpp:2763); the GoldRush bushes carry Size=25610
            // and grow toward FullCon from there.
            "size" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Con `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                let raw = if value > 1000 {
                    value
                } else {
                    (value.clamp(0, 100) * FULL_CON) / 100
                };
                self.construction = Some(raw.max(0));
            }
            "alive" => {
                let alive = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Alive `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.alive = Some(alive);
            }
            // C4Object::InLiquid, persisted with default false
            // (C4Object.cpp:2775) — GoldRush carries InLiquid=1 on its
            // underwater fish and bubbles.
            "inliquid" => {
                let in_liquid = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid InLiquid `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.in_liquid = Some(in_liquid);
            }
            "entrancestatus" => {
                let entrance_status = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid EntranceStatus `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.entrance_status = Some(entrance_status);
            }
            "physicaltemporary" => {
                if self.physical_temporary.is_none() {
                    self.physical_temporary = Some(parse_object_compiler_bool(value));
                }
            }
            "ocf" => {
                self.ocf = Some(parse_object_u32(trimmed_value, self.line, "OCF")?);
            }
            "category" => {
                self.category = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Category `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "dir" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Dir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                // C4Action::CompileFunc persists Dir verbatim without action-
                // range validation (C4Action.cpp:45-54).
                self.direction = Some(Direction::from_raw(raw));
            }
            "comdir" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ComDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                // C4Action::CompileFunc persists ComDir verbatim without
                // COMD_* range validation (C4Action.cpp:45-54).
                self.command_direction = Some(CommandDirection::from_raw(raw));
            }
            "action" => {
                self.action_name = Some(trimmed_value.to_string());
            }
            "actiontime" => {
                let ticks = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTime `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.action_ticks = Some(ticks);
            }
            "phasedelay" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid PhaseDelay `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.action_phase_delay = Some(value);
            }
            "actiondata" => {
                self.action_data = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionData `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "phase" => {
                self.action_phase = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Phase `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "actiontarget1" => {
                self.action_target = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget1 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "actiontarget2" => {
                self.action_target2 = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget2 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "layer" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Layer `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.layer = Some(value);
            }
            "visibility" => {
                self.visibility = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Visibility `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "blitmode" => {
                self.blit_mode = Some(parse_object_u32(trimmed_value, self.line, "BlitMode")?);
            }
            "color" | "colordw" => {
                self.color = Some(parse_object_u32(trimmed_value, self.line, "ColorDw")?);
            }
            "colormod" => {
                self.color_modulation =
                    Some(parse_object_u32(trimmed_value, self.line, "ColorMod")?);
            }
            "picture" => {
                let values = parse_i32_list(trimmed_value, self.line, "Picture")?;
                if values.len() != 4 {
                    return Err(ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: Picture requires 4 integers (got {})",
                        self.line,
                        values.len()
                    )));
                }
                self.picture_rect = Some(DefinitionRect::new(
                    values[0], values[1], values[2], values[3],
                ));
            }
            "plrviewrange" => {
                self.plr_view_range =
                    Some(parse_object_i32(trimmed_value, self.line, "PlrViewRange")?);
            }
            "crewdisabled" => {
                self.crew_disabled =
                    Some(parse_object_bool(trimmed_value, self.line, "CrewDisabled")?);
            }
            "graphics" => {
                self.base_graphics = Some(parse_legacy_object_graphics(
                    trimmed_value,
                    self.line,
                    "Graphics",
                )?);
            }
            "drawtransform" => {
                self.draw_transform = Some(parse_legacy_draw_transform(
                    trimmed_value,
                    self.line,
                    "DrawTransform",
                )?);
            }
            "effects" => {
                self.effects = Some(parse_legacy_object_effects(trimmed_value, self.line)?);
            }
            "gfxoverlay" => {
                self.graphics_overlays =
                    Some(parse_legacy_graphics_overlays(trimmed_value, self.line)?);
            }
            "component" => {
                self.components = Some(parse_legacy_object_components(trimmed_value, self.line)?);
            }
            "contained" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Contained `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.contained = Some(value);
            }
            "contents" => {
                let mut entries = Vec::new();
                for token in trimmed_value.split(';') {
                    let candidate = token.trim();
                    if candidate.is_empty() {
                        continue;
                    }
                    let value = parse_i64(candidate).map_err(|err| {
                        ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {}: invalid Contents entry `{}` ({})",
                            self.line, candidate, err
                        ))
                    })?;
                    if value > 0 {
                        entries.push(value as u64);
                    }
                }
                self.contents = entries;
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_physical_section(&mut self) {
        if self.physical_temporary == Some(true) {
            self.temporary_physical
                .get_or_insert_with(crate::PhysicalInfo::default);
        }
    }

    fn apply_physical_property(
        &mut self,
        key: &str,
        value: &str,
        _line: usize,
    ) -> Result<(), ScenarioError> {
        // C4Object::CompileFunc never follows this sibling section while the
        // flag is false or absent. Its contents are unused namings, including
        // malformed values, rather than parse errors.
        if self.physical_temporary != Some(true) {
            return Ok(());
        }
        if key != "Changes" && !is_legacy_physical_name(key) {
            return Ok(());
        }
        if !self.physical_fields_seen.insert(key.to_string()) {
            return Ok(());
        }
        if key == "Changes" {
            self.physical_changes = parse_legacy_physical_changes(value);
            return Ok(());
        }

        let physical = self
            .temporary_physical
            .get_or_insert_with(crate::PhysicalInfo::default);
        let parsed = match key {
            "Energy" | "Breath" | "Walk" | "Jump" | "Scale" | "Hangle" | "Dig" | "Swim"
            | "Throw" | "Push" | "Fight" | "Magic" | "Float" | "CanScale" | "CanHangle"
            | "CanDig" | "CanConstruct" | "CanChop" | "CanFly" | "CorrosionResist"
            | "BreatheWater" => {
                // Every physical field is wrapped in mkNamingAdapt(..., 0).
                // A malformed first naming is consumed and defaults to zero.
                parse_std_i32(value).unwrap_or_default()
            }
            _ => unreachable!("unknown physical names returned before parsing"),
        };
        match key {
            "Energy" => physical.energy = parsed,
            "Breath" => physical.breath = parsed,
            "Walk" => physical.walk = parsed,
            "Jump" => physical.jump = parsed,
            "Scale" => physical.scale = parsed,
            "Hangle" => physical.hangle = parsed,
            "Dig" => physical.dig = parsed,
            "Swim" => physical.swim = parsed,
            "Throw" => physical.throw = parsed,
            "Push" => physical.push = parsed,
            "Fight" => physical.fight = parsed,
            "Magic" => physical.magic = parsed,
            "Float" => physical.float = parsed,
            "CanScale" => physical.can_scale = parsed,
            "CanHangle" => physical.can_hangle = parsed,
            "CanDig" => physical.can_dig = parsed,
            "CanConstruct" => physical.can_construct = parsed,
            "CanChop" => physical.can_chop = parsed,
            "CanFly" => physical.can_fly = parsed,
            "CorrosionResist" => physical.corrosion_resist = parsed,
            "BreatheWater" => physical.breathe_water = parsed,
            _ => unreachable!("all recognized physical names are assigned"),
        }
        Ok(())
    }

    fn apply_command_property(
        &mut self,
        key: &str,
        value: &str,
        line: usize,
    ) -> Result<(), ScenarioError> {
        let Some(index) = key.strip_prefix("Command") else {
            return Ok(());
        };
        let Ok(index) = index.parse::<usize>() else {
            return Ok(());
        };
        if index == 0 || key != format!("Command{index}") {
            return Ok(());
        }
        let command = parse_legacy_object_command(value, line)?;
        self.commands.insert(index, command);
        Ok(())
    }

    pub(in crate::scenario) fn into_spawn(
        self,
        definition_ids: &HashSet<&str>,
        value_resolution: &SerializedC4ValueResolution<'_>,
    ) -> Result<Option<ScenarioSpawn>, ScenarioError> {
        let Self {
            line,
            id,
            number,
            custom_name,
            info_name,
            status,
            owner,
            controller,
            last_energy_loss_cause,
            x,
            y,
            motion_x,
            motion_y,
            last_attach_movement_frame,
            no_collect_delay,
            base,
            xdir,
            ydir,
            fix_x,
            fix_y,
            fix_r,
            rdir,
            mobile,
            solid_mask,
            rotation,
            timer,
            locals,
            local_named,
            vertex_count,
            vertex_x,
            vertex_y,
            vertex_cnat,
            vertex_friction,
            shape_width,
            shape_height,
            shape_offset,
            shape_fire_top,
            own_vertices,
            contact_density,
            shape_attach_x,
            shape_attach_y,
            shape_attach_vertex,
            own_mass,
            mass,
            damage,
            energy,
            need_energy,
            selected,
            magic_energy,
            construction,
            alive,
            breath,
            fire_phase,
            on_fire,
            in_liquid,
            entrance_status,
            physical_temporary,
            ocf,
            category,
            direction,
            command_direction,
            action_name,
            action_phase,
            action_ticks,
            action_phase_delay,
            action_data,
            action_target,
            action_target2,
            layer,
            visibility,
            blit_mode,
            color,
            color_modulation,
            picture_rect,
            plr_view_range,
            crew_disabled,
            base_graphics,
            draw_transform,
            effects,
            graphics_overlays,
            temporary_physical,
            physical_changes,
            physical_fields_seen: _,
            commands,
            components,
            contained,
            inferred_container,
            contents,
        } = self;

        let id = id.ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: object missing `id`",
                line
            ))
        })?;

        if !definition_ids.contains(id.as_str()) {
            // C++ resolves each Objects.txt entry with C4Id2Def: an unknown
            // id produces no object (logged) and the load continues.
            tracing::warn!(
                definition = %id,
                line,
                "Objects.txt references an unknown definition; skipping the object"
            );
            return Ok(None);
        }

        let number = number.ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: object `{}` missing `Number`",
                line, id
            ))
        })?;

        if matches!(status, Some(ObjectStatus::Deleted)) {
            return Ok(None);
        }

        let mut config = SpawnConfig::new(id.clone())
            .with_id(ObjectId::new(number))
            // Objects.txt entries are LOADED, not created: no
            // Construction/Initialize (C4GameObjects.cpp:535-618).
            .with_loaded(true)
            .with_native_compiled_object_defaults();
        config.compiler_cache = crate::ObjectCompilerCache {
            info: info_name.clone().unwrap_or_default(),
            contained: contained.unwrap_or(0),
            action_target1: action_target.unwrap_or(0),
            action_target2: action_target2.unwrap_or(0),
            layer: layer.unwrap_or(0),
        };
        let offset = shape_offset.unwrap_or_default();
        config = config
            .with_shape_rect(crate::DefinitionRect::new(
                offset.first().copied().unwrap_or(0),
                offset.get(1).copied().unwrap_or(0),
                shape_width.unwrap_or(0),
                shape_height.unwrap_or(0),
            ))
            .with_shape_fire_top(shape_fire_top.unwrap_or(0));
        config = config.with_position(Vector2::new(x.unwrap_or(0), y.unwrap_or(0)));
        config.motion_x = motion_x.unwrap_or(0);
        config.motion_y = motion_y.unwrap_or(0);
        if let Some(custom_name) = custom_name {
            config = config.with_custom_name(custom_name);
        }
        if let Some(layer) = layer
            .and_then(|layer| u64::try_from(layer).ok())
            .filter(|layer| *layer != 0)
        {
            config = config.with_layer(ObjectId::new(layer));
        }
        if let Some(visibility) = visibility {
            config = config.with_visibility(visibility);
        }
        if let Some(blit_mode) = blit_mode {
            config = config.with_blit_mode(blit_mode);
        }
        if let Some(color) = color {
            config = config.with_color(color);
        }
        if let Some(color_modulation) = color_modulation {
            config = config.with_color_modulation(color_modulation);
        }
        if let Some(picture_rect) = picture_rect {
            config = config.with_picture_rect(picture_rect);
        }
        if let Some(components) = components {
            config = config.with_ordered_components(components);
        }
        if let Some(contact_density) = contact_density {
            config = config.with_contact_density(contact_density);
        }
        config.damage = damage;
        config.breath = breath;
        config.own_mass = own_mass;
        config.compiled_mass = mass;
        config.on_fire = on_fire;
        config.fire_phase = fire_phase;
        config.last_attach_movement_frame = last_attach_movement_frame;
        config.last_energy_loss_cause = last_energy_loss_cause;
        config.no_collect_delay = no_collect_delay;
        config.base = base;
        config.compiled_ocf = ocf;
        config.crew_disabled = crew_disabled;
        config.plr_view_range = plr_view_range;
        config.base_graphics = base_graphics;
        config.draw_transform = draw_transform;
        config.graphics_overlays = graphics_overlays
            .unwrap_or_default()
            .into_iter()
            .map(|overlay| overlay.resolve(value_resolution.object_numbers))
            .collect();
        if shape_attach_x.is_some() || shape_attach_y.is_some() || shape_attach_vertex.is_some() {
            config.shape_attach = Some(crate::ShapeAttachRecord {
                // AttachMat is deliberately not compiled by C4Shape.
                mat_valid: false,
                mat_vehicle: false,
                x: shape_attach_x.unwrap_or(0),
                y: shape_attach_y.unwrap_or(0),
                vtx: shape_attach_vertex.unwrap_or(0),
            });
        }

        let resolved_effects = effects
            .unwrap_or_default()
            .into_iter()
            .map(|effect| effect.resolve(value_resolution))
            .collect::<Vec<_>>();
        config.fire_caused_by = Some(
            resolved_effects
                .iter()
                .find(|effect| effect.name == crate::C4FX_FIRE)
                .and_then(|effect| effect.vars.get(1))
                .and_then(|value| match value {
                    EffectVarValue::Int(value) => Some(*value),
                    EffectVarValue::Bool(value) => Some(i32::from(*value)),
                    EffectVarValue::RawBool(value) => Some(*value as u32 as i32),
                    _ => None,
                })
                .unwrap_or(crate::OWNER_NONE),
        );
        config.effects = resolved_effects;

        if physical_temporary.unwrap_or(false) {
            config.temporary_physical = Some(temporary_physical.unwrap_or_default());
            config.physical_changes = physical_changes;
        }

        let mut commands = commands.into_iter().peekable();
        let mut expected_command = 1usize;
        let mut resolved_commands = Vec::new();
        while commands
            .peek()
            .is_some_and(|(index, _)| *index == expected_command)
        {
            let (_, command) = commands.next().expect("peeked command exists");
            resolved_commands.push(command.resolve(line, value_resolution)?);
            expected_command += 1;
        }
        if !resolved_commands.is_empty() {
            config.command_stack = Some(
                crate::command::CommandStackSnapshot::from_legacy_save_commands(resolved_commands)
                    .map_err(|error| {
                        ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {line}: invalid [Commands] stack ({error:?})"
                        ))
                    })?,
            );
        }

        if xdir.is_some() || ydir.is_some() {
            // Exact C4Fixed velocity (C4Object.cpp:2765-2766); the pixel
            // mirror follows fixtoi like C4Object::velocity_pixels.
            let fixed = crate::math::FixedVec2 {
                x: xdir.unwrap_or_default(),
                y: ydir.unwrap_or_default(),
            };
            config = config
                .with_velocity(Vector2::new(
                    crate::math::fixtoi(fixed.x),
                    crate::math::fixtoi(fixed.y),
                ))
                .with_fixed_velocity(fixed);
        }
        // Exact sub-pixel position (FixX/FixY, C4Object.cpp:2762-2763).
        // C++ keeps integer X/Y and fixed coords independent after load;
        // each missing naming value compiles as Fix0. Supplying the zero pair
        // is observable for inactive rows, which never receive SyncClearance.
        config = config.with_fixed_position(crate::math::FixedVec2 {
            x: fix_x.unwrap_or_default(),
            y: fix_y.unwrap_or_default(),
        });
        if let Some(rotation) = rotation {
            config = config.with_rotation(rotation);
        }
        if let Some(fix_r) = fix_r {
            config = config.with_fixed_rotation(fix_r);
        }
        if let Some(rdir) = rdir {
            config = config.with_rotation_velocity(rdir);
        }
        // Loaded objects keep the serialized Mobile verbatim (default
        // false) — they bypass Init, and nothing after C4GameObjects::Load
        // rewrites the flag (C4Object.cpp:2772).
        config = config.with_mobile(mobile.unwrap_or(false));
        if let Some(timer) = timer {
            config = config.with_timer(timer);
        }
        let mut local_vars = HashMap::new();
        if let Some(locals) = locals {
            // C4ValueList slots and named locals are denumerated only after
            // every object exists (C4GameObjects.cpp:600-608).
            local_vars.extend(locals.into_iter().enumerate().map(|(index, value)| {
                (format!("__local_{index}"), value.resolve(value_resolution))
            }));
        }
        if let Some(local_named) = local_named {
            local_vars.extend(
                local_named
                    .into_iter()
                    .map(|(name, value)| (name, value.resolve(value_resolution))),
            );
        }
        if !local_vars.is_empty() {
            config = config.with_local_vars(local_vars);
        }
        // The saved shape's vertices (C4Shape::CompileFunc into [Object],
        // C4Shape.cpp:495-515): the CURRENT effective shape, loaded
        // verbatim (spawn_single skips the Con/rotation re-transform for
        // loaded vertices). Missing arrays read as 0 (mkArrayAdapt).
        // C4Object::Clear zeroes the complete shape before Objects.txt is
        // compiled, so a missing `Vertices` is an explicit VtxNum=0 rather
        // than "fall back to the definition". mkArrayAdapt independently
        // compiles all 30 slots and may retain nonzero dormant values beyond
        // VtxNum (notably own-vertex backups at slots 15+).
        let vertex_count = vertex_count.unwrap_or(0).clamp(0, 30) as usize;
        let component = |list: &Option<Vec<i32>>, index: usize| {
            list.as_ref()
                .and_then(|values| values.get(index).copied())
                .unwrap_or(0)
        };
        let vertex_slots: Vec<crate::ObjectVertex> = (0..30)
            .map(|index| {
                crate::ObjectVertex::new(component(&vertex_x, index), component(&vertex_y, index))
                    .with_cnat(component(&vertex_cnat, index) as u32)
                    .with_friction(component(&vertex_friction, index))
            })
            .collect();
        config = config
            .with_vertices(vertex_slots[..vertex_count].to_vec())
            .with_shape_vertex_slots(vertex_count, vertex_slots);
        if let Some(own_vertices) = own_vertices {
            config = config.with_owns_shape_vertices(own_vertices);
        }
        if let Some(owner) = owner {
            config = config.with_owner(owner);
        }
        if let Some(controller) = controller {
            config = config.with_controller(controller);
        }
        if let Some(energy) = energy {
            config = config.with_energy(energy);
        }
        if let Some(need_energy) = need_energy {
            config = config.with_need_energy(need_energy);
        }
        if let Some(selected) = selected {
            config = config.with_selected(selected);
        }
        if let Some(magic_energy) = magic_energy {
            config = config.with_magic_energy(magic_energy);
        }
        // C4Object::Clear initializes Con to zero before compilation;
        // Objects.txt omits Size only when that exact zero is intended.
        config = config.with_construction(construction.unwrap_or(0));
        if let Some(alive) = alive {
            config = config.with_alive(alive);
        }
        if let Some(in_liquid) = in_liquid {
            config = config.with_in_liquid(in_liquid);
        }
        if let Some(entrance_status) = entrance_status {
            config = config.with_entrance_status(entrance_status);
        }
        if let Some(category) = category {
            config = config.with_category(category);
        }
        if let Some(status) = status {
            if status != ObjectStatus::Normal {
                config = config.with_status(status);
            }
        }
        if let Some(values) = solid_mask {
            let mut it = values.into_iter().chain(std::iter::repeat(0));
            let rect = crate::DefinitionTargetRect::new(
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
            );
            config = config.with_solid_mask(rect);
        }
        if let Some(direction) = direction {
            config = config.with_direction(direction);
        }
        if let Some(command_direction) = command_direction {
            config = config.with_command_direction(command_direction);
        }
        if let Some(action_state) = build_action_state(
            action_name,
            action_phase,
            action_ticks,
            action_phase_delay,
            action_data,
            action_target,
            action_target2,
        ) {
            config = config.with_action(action_state);
        }

        let container_handle = contained
            .map(|number| {
                if (1_000_000_000..=1_001_000_000).contains(&number) {
                    number - 1_000_000_000
                } else {
                    number
                }
            })
            .filter(|number| *number > 0)
            .map(|number| number.to_string())
            .or_else(|| inferred_container.map(|number| number.to_string()));
        Ok(Some(ScenarioSpawn {
            handle: Some(number.to_string()),
            container_handle,
            contents_handles: contents
                .into_iter()
                .map(|value| value.to_string())
                .collect(),
            info_name,
            config,
        }))
    }
}

pub(in crate::scenario) fn build_action_state(
    name: Option<String>,
    phase: Option<i32>,
    time: Option<i32>,
    phase_delay: Option<i32>,
    data: Option<i32>,
    target: Option<i32>,
    target2: Option<i32>,
) -> Option<ActionState> {
    if name.is_none()
        && phase.is_none()
        && time.is_none()
        && phase_delay.is_none()
        && data.is_none()
        && target.is_none()
        && target2.is_none()
    {
        return None;
    }
    // C4Action::CompileFunc compiles every field independently. A save may
    // carry ActionTarget1/2 without an explicit Action name; its zeroed
    // fixed-size Name buffer is an empty string. SetActionByName("") then
    // fails, preserving the saved fixed coordinates, while the pointers
    // still proceed through DenumeratePointers.
    // `C4Action::Name` is a `C4MaxName + 1` fixed buffer compiled through
    // `toC4CStr` (C4Action.cpp:45-54). Both lookup and a failed lookup's
    // observable raw name therefore see at most 30 native bytes, not 30
    // Unicode scalar values. Round-trip through the C4 byte projection so
    // legacy high bytes are neither split nor counted as UTF-8 characters.
    let name = name.unwrap_or_default();
    let name_bytes = clonk_script::c4_string_bytes(&name);
    let visible_len = name_bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(name_bytes.len())
        .min(30);
    let name = clonk_script::c4_string_from_bytes(&name_bytes[..visible_len]);
    let name = if is_builtin_idle_name(&name) {
        "Idle".to_string()
    } else {
        name
    };
    let mut state = ActionState::new(name);
    if let Some(value) = phase {
        state.phase = value;
    }
    // ActionTime= is Action.Time; PhaseDelay= is the intra-phase counter
    // (C4Object.cpp:2840-2849 restores Time/Phase/PhaseDelay verbatim).
    if let Some(value) = time {
        state.time = value;
    }
    if let Some(value) = phase_delay {
        state.ticks = value;
    }
    if let Some(value) = data {
        state.data = value;
    }
    if let Some(target) = target.and_then(|target| u64::try_from(target).ok()) {
        state.target = Some(ObjectId::new(target));
    }
    if let Some(target2) = target2.and_then(|target| u64::try_from(target).ok()) {
        state.target2 = Some(ObjectId::new(target2));
    }
    Some(state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyObjectParseSection {
    Object,
    Physical,
    Commands,
    Other,
}

fn parse_legacy_ini_section_name(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'[') || !bytes.get(1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut position = 1usize;
    while bytes
        .get(position)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
    {
        position += 1;
    }
    let name_end = position;
    while bytes
        .get(position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        position += 1;
    }
    (bytes.get(position) == Some(&b']')).then(|| &line[1..name_end])
}

fn parse_legacy_ini_property(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut position = 0usize;
    while bytes
        .get(position)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
    {
        position += 1;
    }
    let name_end = position;
    while bytes
        .get(position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        position += 1;
    }
    if bytes.get(position) != Some(&b'=') {
        return None;
    }
    Some((&line[..name_end], &line[position + 1..]))
}

pub(in crate::scenario) fn parse_legacy_objects(
    text: &str,
) -> Result<Vec<LegacyObjectRecord>, ScenarioError> {
    let mut records = Vec::new();
    let mut current: Option<LegacyObjectRecord> = None;
    let mut section_stack: Vec<(usize, LegacyObjectParseSection)> = Vec::new();
    let mut object_indent = None;
    // FollowName("Physical") only sees the next sibling of [Object]. A child
    // section does not consume that position, but a same-level (or outer)
    // section does, even when its name is otherwise unknown.
    let mut physical_may_follow_object = false;

    for (index, raw_line) in text.lines().enumerate() {
        // StdCompilerINIRead does not have an inline-comment syntax. In
        // particular, `//` inside RCT_All values (Info and command Text) is
        // ordinary persisted data. Retain the right-hand end of every line.
        let raw_line = raw_line.trim_start_matches('\u{feff}');
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let line = raw_line.trim_start_matches([' ', '\t']);
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        if let Some(section_name) = parse_legacy_ini_section_name(line) {
            while section_stack
                .last()
                .is_some_and(|(section_indent, _)| *section_indent >= indent)
            {
                section_stack.pop();
            }
            let has_parent_section = !section_stack.is_empty();
            // Only [Object] creates a row. Nested naming environments belong
            // to that row and must route their properties to their own
            // compiler instead of falling through to C4Object::CompileFunc.
            let parsed_section = if section_name == "Object" && !has_parent_section {
                if let Some(record) = current.take() {
                    records.push(record);
                }
                current = Some(LegacyObjectRecord::new(index + 1));
                object_indent = Some(indent);
                physical_may_follow_object = true;
                LegacyObjectParseSection::Object
            } else if section_name == "Physical"
                && current.is_some()
                && !has_parent_section
                && physical_may_follow_object
            {
                if let Some(record) = current.as_mut() {
                    record.begin_physical_section();
                }
                physical_may_follow_object = false;
                LegacyObjectParseSection::Physical
            } else if section_name == "Commands" {
                if object_indent.is_some_and(|object_indent| indent <= object_indent) {
                    physical_may_follow_object = false;
                }
                LegacyObjectParseSection::Commands
            } else {
                if object_indent.is_some_and(|object_indent| indent <= object_indent) {
                    physical_may_follow_object = false;
                }
                LegacyObjectParseSection::Other
            };
            section_stack.push((indent, parsed_section));
            continue;
        }
        let Some((key, value)) = parse_legacy_ini_property(line) else {
            continue;
        };

        // Native INI values receive one implicit indentation level. Pop any
        // child sections the value has left, revealing its enclosing naming.
        while section_stack
            .last()
            .is_some_and(|(section_indent, _)| *section_indent > indent)
        {
            section_stack.pop();
        }
        let section = section_stack
            .last()
            .map_or(LegacyObjectParseSection::Other, |(_, section)| *section);
        if section == LegacyObjectParseSection::Other
            && object_indent.is_some_and(|object_indent| indent < object_indent)
        {
            physical_may_follow_object = false;
        }
        match section {
            LegacyObjectParseSection::Object => {
                let record = current.as_mut().expect("Object section creates a record");
                record.apply_property(key, value)?;
            }
            LegacyObjectParseSection::Physical => {
                if let Some(record) = current.as_mut() {
                    record.apply_physical_property(key, value, index + 1)?;
                }
            }
            LegacyObjectParseSection::Commands => {
                if let Some(record) = current.as_mut() {
                    record.apply_command_property(key, value, index + 1)?;
                }
            }
            LegacyObjectParseSection::Other => {}
        }
    }

    if let Some(record) = current.take() {
        records.push(record);
    }

    Ok(records)
}

fn object_property_error(line: usize, key: &str, value: &str, detail: &str) -> ScenarioError {
    ScenarioError::LegacyObjectsParse(format!(
        "Objects.txt line {line}: invalid {key} `{value}` ({detail})"
    ))
}

fn parse_object_i32(value: &str, line: usize, key: &str) -> Result<i32, ScenarioError> {
    parse_i32(value).map_err(|error| object_property_error(line, key, value, &error))
}

fn parse_object_u32(value: &str, line: usize, key: &str) -> Result<u32, ScenarioError> {
    // StdCompilerINIRead reads unsigned fields through strtoul and then
    // stores the low uint32 word. In particular, older Objects.txt files
    // spell high-bit OCF/colour values as signed decimal numbers. Preserve
    // those bits instead of rejecting the leading minus sign.
    parse_std_u32(value)
        .ok_or_else(|| object_property_error(line, key, value, "invalid uint32 value"))
}

fn parse_object_bool(value: &str, line: usize, key: &str) -> Result<bool, ScenarioError> {
    parse_bool(value)
        .ok_or_else(|| object_property_error(line, key, value, "expected a boolean value"))
}

/// StdCompilerINIRead::Boolean reads directly after `=` without skipping
/// whitespace. It accepts the exact lowercase prefixes `true` and `false`, or
/// a leading 0/1 not followed by another digit. Invalid input is caught by the
/// surrounding default adaptor and becomes false.
fn parse_object_compiler_bool(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'1') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        true
    } else if bytes.first() == Some(&b'0') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        false
    } else {
        value.starts_with("true")
    }
}

fn parse_legacy_object_graphics(
    value: &str,
    line: usize,
    key: &str,
) -> Result<crate::ObjectBaseGraphics, ScenarioError> {
    let Some((definition, graphics_name)) = value.split_once("::") else {
        return Err(object_property_error(
            line,
            key,
            value,
            "expected DEFN::GraphicsName",
        ));
    };
    let definition = definition.trim();
    if clonk_script::c4_string_bytes(definition).len() != 4
        || clonk_script::c4_id_raw(definition) == 0
    {
        return Err(object_property_error(
            line,
            key,
            value,
            "definition id must contain exactly four native C4 bytes",
        ));
    }
    let graphics_name = graphics_name.trim();
    Ok(crate::ObjectBaseGraphics {
        definition: definition.to_string(),
        graphics_name: (!graphics_name.is_empty()).then(|| graphics_name.to_string()),
        // C4DefGraphicsAdapt contains only the definition/name pair. The
        // object's independent BlitMode field is compiled elsewhere.
        blit_mode: 0,
    })
}

fn parse_legacy_draw_transform(
    value: &str,
    line: usize,
    key: &str,
) -> Result<crate::DrawTransform, ScenarioError> {
    let fields = split_outside_delimiter(value, ',');
    if !(7..=10).contains(&fields.len()) {
        return Err(object_property_error(
            line,
            key,
            value,
            &format!(
                "expected six affine values, FlipDir, and up to three projective values; found {} fields",
                fields.len()
            ),
        ));
    }
    let mut matrix = [0.0_f32; 9];
    matrix[8] = 1.0;
    for (index, field) in fields.iter().take(6).enumerate() {
        matrix[index] = field.trim().parse::<f32>().map_err(|error| {
            object_property_error(
                line,
                key,
                value,
                &format!("invalid matrix component {}: {error}", index + 1),
            )
        })?;
    }
    let flip_dir = parse_object_i32(fields[6].trim(), line, key)?;
    for (offset, field) in fields.iter().skip(7).enumerate() {
        matrix[6 + offset] = field.trim().parse::<f32>().map_err(|error| {
            object_property_error(
                line,
                key,
                value,
                &format!("invalid projective component {}: {error}", offset + 1),
            )
        })?;
    }
    Ok(crate::DrawTransform::from_matrix_with_flip_dir(
        matrix, flip_dir,
    ))
}

fn parse_legacy_graphics_overlays(
    value: &str,
    line: usize,
) -> Result<Vec<SerializedObjectGraphicsOverlay>, ScenarioError> {
    split_outside_delimiter(value, ';')
        .into_iter()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| parse_legacy_graphics_overlay(entry, line))
        .collect()
}

fn parse_legacy_graphics_overlay(
    value: &str,
    line: usize,
) -> Result<SerializedObjectGraphicsOverlay, ScenarioError> {
    let fields = split_outside_delimiter(value, ',');
    if !(7..=9).contains(&fields.len()) {
        return Err(object_property_error(
            line,
            "GfxOverlay",
            value,
            &format!("expected 7 to 9 fields, found {}", fields.len()),
        ));
    }
    let graphics = fields[1].trim();
    let graphics = if graphics.is_empty() {
        None
    } else {
        Some(parse_legacy_object_graphics(
            graphics,
            line,
            "GfxOverlay graphics",
        )?)
    };
    let mode_value = parse_object_i32(fields[2].trim(), line, "GfxOverlay mode")?;
    let mode = crate::GraphicsOverlayMode::from_script_value(mode_value).ok_or_else(|| {
        object_property_error(
            line,
            "GfxOverlay mode",
            fields[2].trim(),
            "unsupported graphics-overlay mode",
        )
    })?;
    let transform = fields[6]
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| {
            object_property_error(
                line,
                "GfxOverlay transform",
                fields[6].trim(),
                "expected a parenthesized draw transform",
            )
        })?;
    Ok(SerializedObjectGraphicsOverlay {
        id: parse_object_i32(fields[0].trim(), line, "GfxOverlay id")?,
        mode,
        definition: graphics
            .as_ref()
            .map(|graphics| graphics.definition.clone()),
        graphics_name: graphics.and_then(|graphics| graphics.graphics_name),
        action: (!fields[3].trim().is_empty()).then(|| fields[3].trim().to_string()),
        blit_mode: parse_object_u32(fields[4].trim(), line, "GfxOverlay blit mode")?,
        phase: parse_object_i32(fields[5].trim(), line, "GfxOverlay phase")?,
        transform: parse_legacy_draw_transform(transform, line, "GfxOverlay transform")?,
        color_modulation: if fields.len() >= 8 {
            parse_object_u32(fields[7].trim(), line, "GfxOverlay color modulation")?
        } else {
            0x00ff_ffff
        },
        overlay_object: if fields.len() >= 9 {
            parse_object_i32(fields[8].trim(), line, "GfxOverlay object")?
        } else {
            0
        },
    })
}

fn parse_legacy_physical_changes(value: &str) -> Vec<(String, i32)> {
    let mut changes = Vec::new();
    let mut position = 0usize;
    loop {
        skip_std_whitespace(value, &mut position);
        let name_start = position;
        while value
            .as_bytes()
            .get(position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        {
            position += 1;
        }
        if position == name_start {
            break;
        }
        let name = &value[name_start..position];
        if !is_legacy_physical_name(name) {
            break;
        }
        if !consume_std_separator(value, &mut position, b'=') {
            break;
        }
        let Some(previous) = parse_std_i32_prefix_at(value, &mut position) else {
            break;
        };
        changes.push((name.to_string(), previous));
        if !consume_std_separator(value, &mut position, b',') {
            break;
        }
    }
    changes
}

fn is_legacy_physical_name(name: &str) -> bool {
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

pub(in crate::scenario) fn parse_legacy_object_command(
    value: &str,
    line: usize,
) -> Result<SerializedLegacyCommand, ScenarioError> {
    let value = value.trim_start();
    let (version, payload) = if let Some(versioned) = value.strip_prefix('$') {
        let (version, payload) = versioned.split_once(',').ok_or_else(|| {
            object_property_error(
                line,
                "Command",
                value,
                "versioned command is missing its first separator",
            )
        })?;
        let version = parse_object_i32(version.trim(), line, "Command version")?;
        (version, payload)
    } else {
        (0, value)
    };

    // Version zero has no BaseMode field. Versions one and later do. The
    // final RCT_All text field may itself contain commas, so cap the split at
    // the layout's exact field count.
    let field_count = if version > 0 { 15 } else { 14 };
    let fields = split_outside_delimiter_limit(payload, ',', field_count);
    if fields.len() != field_count {
        return Err(object_property_error(
            line,
            "Command",
            value,
            &format!(
                "command version {version} requires {field_count} payload fields, found {}",
                fields.len()
            ),
        ));
    }
    let name = fields[0].trim();
    if crate::command::CommandId::from_name(name).is_none() {
        return Err(object_property_error(
            line,
            "Command name",
            name,
            "unknown C4 command",
        ));
    }
    let integer = |index: usize, label: &str| parse_object_i32(fields[index].trim(), line, label);
    let base_mode = if version > 0 {
        integer(13, "Command BaseMode")?
    } else {
        0
    };
    let text_index = if version > 0 { 14 } else { 13 };
    let mut text = fields[text_index].to_string();
    // C4Command::CompileFunc's compatibility repair for old layouts.
    if version < 2 && text == "0" {
        text.clear();
    }
    Ok(SerializedLegacyCommand {
        name: name.to_string(),
        tx: parse_serialized_c4value(fields[1].trim(), line)?,
        ty: integer(2, "Command Ty")?,
        target: integer(3, "Command Target")?,
        target2: integer(4, "Command Target2")?,
        data: integer(5, "Command Data")?,
        update_interval: integer(6, "Command UpdateInterval")?,
        evaluated: integer(7, "Command Evaluated")?,
        path_checked: integer(8, "Command PathChecked")?,
        finished: integer(9, "Command Finished")?,
        failures: integer(10, "Command Failures")?,
        retries: integer(11, "Command Retries")?,
        permit: integer(12, "Command Permit")?,
        base_mode,
        // RCT_All consumes the complete remaining field, including commas.
        text,
    })
}

fn parse_legacy_object_effects(
    value: &str,
    line: usize,
) -> Result<Vec<SerializedEffectState>, ScenarioError> {
    split_outside_delimiter(value, ',')
        .into_iter()
        .map(str::trim)
        .filter(|effect| !effect.is_empty())
        .map(|effect| {
            parse_serialized_effect_state(effect, line)
                .map_err(|detail| object_property_error(line, "Effects", effect, detail.as_str()))
        })
        .collect()
}

/// C4Object::CustomName uses StdCompiler's escaped-string adapter. Modern
/// saves quote the value; older shipped saves keep the whole unquoted line
/// (StdCompiler.cpp:734-741, 936-976, 1006-1062).
pub(in crate::scenario) fn parse_legacy_object_name(
    value: &str,
    line: usize,
) -> Result<Option<String>, ScenarioError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !trimmed.starts_with('"') {
        return Ok(Some(trimmed.to_string()));
    }

    let mut chars = trimmed[1..].chars().peekable();
    let mut decoded = String::new();
    let mut terminated = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                terminated = true;
                break;
            }
            '\\' => {
                let escaped = match chars.next() {
                    Some('a') => '\u{0007}',
                    Some('b') => '\u{0008}',
                    Some('f') => '\u{000c}',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('v') => '\u{000b}',
                    Some('\'') => '\'',
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('?') => '?',
                    Some('x') => {
                        let mut code = 0u32;
                        let mut found = false;
                        while let Some(digit) = chars.peek().and_then(|next| next.to_digit(16)) {
                            found = true;
                            code = code.wrapping_mul(16).wrapping_add(digit);
                            chars.next();
                        }
                        if found {
                            char::from_u32(code & 0xff).unwrap_or('\0')
                        } else {
                            'x'
                        }
                    }
                    Some(first @ '0'..='7') => {
                        let mut code = first.to_digit(8).unwrap_or(0);
                        while let Some(digit) = chars.peek().and_then(|next| next.to_digit(8)) {
                            code = code.wrapping_mul(8).wrapping_add(digit);
                            chars.next();
                        }
                        char::from_u32(code & 0xff).unwrap_or('\0')
                    }
                    Some(other) => other,
                    None => {
                        return Err(ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {}: unterminated escape in Name",
                            line
                        )));
                    }
                };
                decoded.push(escaped);
            }
            other => decoded.push(other),
        }
    }

    if !terminated || chars.any(|ch| !ch.is_whitespace()) {
        return Err(ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: unterminated or malformed quoted Name `{}`",
            line, trimmed
        )));
    }
    Ok((!decoded.is_empty()).then_some(decoded))
}

pub(in crate::scenario) fn parse_i64(value: &str) -> Result<i64, std::num::ParseIntError> {
    let trimmed = value.trim();
    // Handle C4Fixed format: 'f' or 'F' prefix indicates a fixed-point number
    // Strip the prefix and parse the integer value (which may be hex or decimal)
    let trimmed = trimmed
        .strip_prefix('f')
        .or_else(|| trimmed.strip_prefix('F'))
        .unwrap_or(trimmed);

    if let Some(rest) = trimmed
        .strip_prefix("-0x")
        .or_else(|| trimmed.strip_prefix("-0X"))
    {
        i64::from_str_radix(rest, 16).map(|parsed| -parsed)
    } else if let Some(rest) = trimmed
        .strip_prefix("+0x")
        .or_else(|| trimmed.strip_prefix("+0X"))
    {
        i64::from_str_radix(rest, 16)
    } else if let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        i64::from_str_radix(rest, 16)
    } else {
        // StdCompilerINIRead reads numbers strtol-style: optional sign +
        // leading digits, trailing junk ignored (real content carries
        // trailing `;`, e.g. `Position=22,28;` in LastWill.c4s). No digits
        // at all stays an error (the empty-slice parse).
        let (sign, digits) = match trimmed.as_bytes().first() {
            Some(b'-') => (-1i64, &trimmed[1..]),
            Some(b'+') => (1, &trimmed[1..]),
            _ => (1, trimmed),
        };
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        digits[..end].parse::<i64>().map(|value| sign * value)
    }
}

pub(crate) fn parse_i32(value: &str) -> Result<i32, String> {
    let parsed = parse_i64(value).map_err(|err| err.to_string())?;
    i32::try_from(parsed).map_err(|_| "value out of range for i32".to_string())
}

/// Objects.txt `LocalNamed=` (C4ValueMapData::CompileFunc,
/// C4ValueMap.cpp:236-295): `<count>;name=<value>,name=<value>,...` where
/// each value uses the C4Value type-char encoding (GetC4VID,
/// C4Value.cpp:368-394). A zero count writes no separator and no entries.
#[derive(Debug, Default)]
pub(in crate::scenario) struct InitialNetworkRuntimeState {
    pub(in crate::scenario) sky: Option<InitialNetworkSkyState>,
    script_globals: SerializedScriptGlobalState,
    pub(in crate::scenario) global_effects: Vec<SerializedEffectState>,
    pub(in crate::scenario) scoreboard: ScoreboardState,
}

#[derive(Debug)]
pub(in crate::scenario) struct InitialNetworkSkyState {
    fixed: [i32; 4],
    modulation: u32,
    parallax_x: i32,
    parallax_y: i32,
    parallax_mode: i32,
    back_color: u32,
    back_color_enabled: bool,
}

#[derive(Debug, Default)]
struct SerializedScriptGlobalState {
    numbered: Vec<SerializedC4Value>,
    named: Vec<(String, SerializedC4Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::scenario) struct SerializedEffectState {
    pub(in crate::scenario) number: i32,
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) priority: i32,
    pub(in crate::scenario) interval: i32,
    pub(in crate::scenario) timer: i32,
    pub(in crate::scenario) command_target: i32,
    pub(in crate::scenario) command_id: Option<String>,
    pub(in crate::scenario) vars: Vec<SerializedC4Value>,
}

impl InitialNetworkRuntimeState {
    pub(in crate::scenario) fn parse(data: &InitialNetworkGameData) -> Result<Self, ScenarioError> {
        Ok(Self {
            sky: data
                .compiled_sections
                .sky()
                .map(parse_initial_network_sky)
                .transpose()?,
            script_globals: data
                .compiled_sections
                .script_engine()
                .map(parse_initial_network_script_globals)
                .transpose()?
                .unwrap_or_default(),
            global_effects: data
                .compiled_sections
                .effects()
                .map(parse_initial_network_effects)
                .transpose()?
                .unwrap_or_default(),
            scoreboard: data
                .compiled_sections
                .scoreboard()
                .map(parse_initial_network_scoreboard)
                .transpose()?
                .unwrap_or_default(),
        })
    }

    pub(in crate::scenario) fn resolve_post_object_state(
        self,
        object_numbers: &HashSet<u64>,
        string_registrations: &clonk_script::StringRegistrations,
    ) -> (ScriptGlobalState, Vec<EffectState>) {
        let resolution = SerializedC4ValueResolution {
            object_numbers,
            string_registrations,
        };
        let numbered = self
            .script_globals
            .numbered
            .into_iter()
            .enumerate()
            .filter_map(|(index, value)| {
                i32::try_from(index)
                    .ok()
                    .map(|index| (index, value.resolve(&resolution)))
            })
            .collect::<BTreeMap<_, _>>();
        let named = self
            .script_globals
            .named
            .into_iter()
            .map(|(name, value)| (name, value.resolve(&resolution)))
            .collect::<BTreeMap<_, _>>();
        let effects = self
            .global_effects
            .into_iter()
            .map(|effect| effect.resolve(&resolution))
            .collect();
        (ScriptGlobalState { numbered, named }, effects)
    }
}

impl InitialNetworkSkyState {
    /// C4Game compiles the runtime words before C4Landscape::Init calls
    /// C4Sky::Init. Fresh games reset scroll position/speed/parallax there;
    /// savegames retain them. A loaded bitmap then applies SkyScrollMode on
    /// top in both cases (C4Game.cpp:2654-2665; C4Sky.cpp:71-125).
    pub(in crate::scenario) fn into_frame(
        self,
        mut settings: SkySettings,
        savegame: bool,
        sky_scroll_mode: i32,
    ) -> SkyFrame {
        let fixed = if savegame { self.fixed } else { [0; 4] };
        settings.parallax_x = if savegame { self.parallax_x } else { 10 };
        settings.parallax_y = if savegame { self.parallax_y } else { 10 };
        settings.parallax_mode = if savegame && self.parallax_mode == 1 {
            SkyParallaxMode::Wind
        } else {
            SkyParallaxMode::Fixed
        };
        if settings.has_surface {
            match sky_scroll_mode {
                1 => {
                    settings.parallax_mode = SkyParallaxMode::Wind;
                    settings.parallax_y = 20;
                }
                2 => {
                    settings.parallax_x = 20;
                    settings.parallax_y = 20;
                }
                _ => {}
            }
        }
        settings.modulation = Some(self.modulation);
        settings.back_color_raw = self.back_color;
        settings.back_color = self.back_color_enabled.then_some(self.back_color);
        settings.base_xdir = crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[2]));
        settings.base_ydir = crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[3]));
        SkyFrame {
            settings,
            offset_x: crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[0])),
            offset_y: crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[1])),
            fixed: Some(fixed),
        }
    }
}

impl SerializedEffectState {
    pub(in crate::scenario) fn resolve(
        self,
        resolution: &SerializedC4ValueResolution<'_>,
    ) -> EffectState {
        // C4EnumeratedObjectPtr only recognizes the old pointer-offset
        // spelling inside the complete C4EnumPointer1..C4EnumPointer2 range.
        // A modern, raw object number above that range must not be shifted
        // (C4EnumeratedObjectPtr.cpp:32-42).
        let command_target = if (1_000_000_000..=1_001_000_000).contains(&self.command_target) {
            self.command_target - 1_000_000_000
        } else {
            self.command_target
        };
        let command_target = u64::try_from(command_target)
            .ok()
            .filter(|number| *number != 0 && resolution.object_numbers.contains(number))
            .and_then(|number| i32::try_from(number).ok());
        EffectState {
            number: self.number,
            name: self.name,
            priority: self.priority,
            interval: self.interval,
            timer: self.timer,
            command_target,
            command_id: self.command_id,
            vars: self
                .vars
                .into_iter()
                .map(|value| effect_var_from_value(value.resolve(resolution)))
                .collect(),
            // A compiled effect has already run its synchronous Start call.
            start_dispatched: true,
        }
    }
}

fn initial_network_section_tree(
    bytes: &[u8],
    name: &str,
) -> Result<(LegacyIniTree, usize), ScenarioError> {
    let source = clonk_script::c4_string_from_bytes(bytes);
    let tree = LegacyIniTree::parse(&source);
    let section = tree.first_section(0, name).ok_or_else(|| {
        ScenarioError::InitialNetworkRuntime(format!(
            "retained [{name}] block has no [{name}] section"
        ))
    })?;
    Ok((tree, section))
}

fn parse_initial_network_sky(bytes: &[u8]) -> Result<InitialNetworkSkyState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Sky")?;
    Ok(InitialNetworkSkyState {
        fixed: [
            ini_i32(&tree, section, "X", 0),
            ini_i32(&tree, section, "Y", 0),
            ini_i32(&tree, section, "XDir", 0),
            ini_i32(&tree, section, "YDir", 0),
        ],
        modulation: ini_u32(&tree, section, "Modulation", 0x00ff_ffff),
        parallax_x: ini_i32(&tree, section, "ParX", 10),
        parallax_y: ini_i32(&tree, section, "ParY", 10),
        parallax_mode: ini_i32(&tree, section, "ParMode", 0),
        back_color: ini_u32(&tree, section, "BackClr", 0),
        back_color_enabled: ini_bool(&tree, section, "BackClrEnabled", false),
    })
}

fn parse_initial_network_script_globals(
    bytes: &[u8],
) -> Result<SerializedScriptGlobalState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Script")?;
    let numbered = match tree.value(section, "Globals") {
        Some(value) => parse_local_slots(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script] Globals: {error}"))
        })?,
        None => parse_nested_script_globals(bytes)?,
    };
    let named = match tree.value(section, "GlobalNamed") {
        Some(value) => parse_local_named(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script] GlobalNamed: {error}"))
        })?,
        None => parse_nested_script_global_named(bytes)?,
    };
    Ok(SerializedScriptGlobalState { numbered, named })
}

fn nested_script_entries(bytes: &[u8], target: &str) -> Option<Vec<(String, String)>> {
    let source = clonk_script::c4_string_from_bytes(bytes);
    let mut target_indent = None;
    let mut entries = Vec::new();
    for line in legacy_ini_lines(&source) {
        let indent = line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let trimmed = line.trim_start_matches([' ', '\t']);
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|value| value.split_once(']'))
            .map(|(name, _)| name)
        {
            if target_indent.is_some_and(|target_indent| indent <= target_indent) {
                break;
            }
            if name == target {
                target_indent = Some(indent);
            }
            continue;
        }
        if target_indent.is_none() {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        entries.push((name.trim().to_string(), value.to_string()));
    }
    target_indent.map(|_| entries)
}

fn parse_nested_script_globals(bytes: &[u8]) -> Result<Vec<SerializedC4Value>, ScenarioError> {
    let Some(entries) = nested_script_entries(bytes, "Globals") else {
        return Ok(Vec::new());
    };
    if let Some((_, value)) = entries
        .iter()
        .find(|(name, _)| matches!(name.as_str(), "Value" | "Values" | "Data"))
    {
        return parse_local_slots(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script][Globals]: {error}"))
        });
    }
    let mut indexed = Vec::new();
    for (name, encoded) in entries {
        let index = name.parse::<usize>().map_err(|_| {
            ScenarioError::InitialNetworkRuntime(format!(
                "[Script][Globals] invalid slot name `{name}`"
            ))
        })?;
        let value = parse_nested_script_c4value(&encoded)?;
        indexed.push((index, value));
    }
    indexed.sort_by_key(|(index, _)| *index);
    let size = indexed
        .last()
        .map_or(0, |(index, _)| index.saturating_add(1));
    let mut values = (0..size)
        .map(|_| SerializedC4Value::Value(clonk_script::Value::Nil))
        .collect::<Vec<_>>();
    for (index, value) in indexed {
        values[index] = value;
    }
    Ok(values)
}

fn parse_nested_script_global_named(
    bytes: &[u8],
) -> Result<Vec<(String, SerializedC4Value)>, ScenarioError> {
    let Some(entries) = nested_script_entries(bytes, "GlobalNamed") else {
        return Ok(Vec::new());
    };
    if let Some((_, value)) = entries
        .iter()
        .find(|(name, _)| matches!(name.as_str(), "Value" | "Values" | "Data"))
    {
        return parse_local_named(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script][GlobalNamed]: {error}"))
        });
    }
    entries
        .into_iter()
        .map(|(name, encoded)| Ok((name, parse_nested_script_c4value(&encoded)?)))
        .collect()
}

fn parse_nested_script_c4value(encoded: &str) -> Result<SerializedC4Value, ScenarioError> {
    let encoded = encoded.trim();
    if encoded.chars().next().is_some_and(|type_char| {
        matches!(
            type_char,
            'A' | 'i' | 'b' | 'o' | 'O' | 'I' | 'S' | 'a' | 'm'
        )
    }) {
        return parse_serialized_c4value(encoded, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("nested Script C4Value: {error}"))
        });
    }
    let value = parse_i32(encoded).map_err(|error| {
        ScenarioError::InitialNetworkRuntime(format!(
            "nested Script value `{encoded}` is neither typed nor an integer ({error})"
        ))
    })?;
    Ok(SerializedC4Value::Value(if value == 0 {
        clonk_script::Value::Nil
    } else {
        clonk_script::Value::Int(value)
    }))
}

fn parse_initial_network_effects(
    bytes: &[u8],
) -> Result<Vec<SerializedEffectState>, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Effects")?;
    let Some(serialized) = tree.value(section, "GlobalEffects") else {
        return Ok(Vec::new());
    };
    split_outside_delimiter(serialized.trim(), ',')
        .into_iter()
        .map(str::trim)
        .filter(|effect| !effect.is_empty())
        .map(parse_initial_network_effect)
        .collect()
}

fn parse_initial_network_effect(serialized: &str) -> Result<SerializedEffectState, ScenarioError> {
    let error = |detail: String| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Effects] GlobalEffects `{serialized}`: {detail}"
        ))
    };
    parse_serialized_effect_state(serialized, 1).map_err(error)
}

/// Shared C4Effect::CompileFunc decoder for global and per-object chains.
/// Its variables remain serialized until all object numbers and Strings.txt
/// entries are available for the native denumeration pass.
fn parse_serialized_effect_state(
    serialized: &str,
    line: usize,
) -> Result<SerializedEffectState, String> {
    let error = |detail: String| format!("effect `{serialized}`: {detail}");
    let open = serialized
        .find('(')
        .ok_or_else(|| error("missing `(`".to_string()))?;
    let close = serialized[open + 1..]
        .find(')')
        .map(|index| open + 1 + index)
        .ok_or_else(|| error("missing `)`".to_string()))?;
    let name = serialized[..open].trim();
    if name.is_empty() {
        return Err(error("missing effect name".to_string()));
    }
    let fields = split_outside_delimiter(&serialized[open + 1..close], ',');
    if fields.len() != 6 {
        return Err(error(format!(
            "expected 6 header fields, found {}",
            fields.len()
        )));
    }
    let int_field = |index: usize, label: &str| {
        parse_std_i32(fields[index])
            .ok_or_else(|| error(format!("invalid {label} value `{}`", fields[index].trim())))
    };
    let command_id = fields[5].trim();
    let command_id = if command_id == "NONE" {
        None
    } else if clonk_script::c4_string_bytes(command_id).len() == 4 {
        Some(command_id.to_string())
    } else {
        None
    };
    let tail = serialized[close + 1..].trim();
    let vars = if tail.is_empty() {
        Vec::new()
    } else {
        let inner = tail
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .ok_or_else(|| error(format!("invalid effect variable list `{tail}`")))?;
        parse_local_slots(inner, line).map_err(|parse_error| error(parse_error.to_string()))?
    };
    Ok(SerializedEffectState {
        name: name.to_string(),
        number: int_field(0, "number")?,
        priority: int_field(1, "priority")?,
        timer: int_field(2, "time")?,
        interval: int_field(3, "interval")?,
        command_target: int_field(4, "command target")?,
        command_id,
        vars,
    })
}

fn parse_initial_network_scoreboard(bytes: &[u8]) -> Result<ScoreboardState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Scoreboard")?;
    let rows = ini_i32(&tree, section, "Rows", 0);
    let columns = ini_i32(&tree, section, "Cols", 0);
    let show_count = ini_i32(&tree, section, "DlgShow", 0);
    let row_count = usize::try_from(rows).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!("[Scoreboard] negative Rows value {rows}"))
    })?;
    let column_count = usize::try_from(columns).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!("[Scoreboard] negative Cols value {columns}"))
    })?;
    let cell_count = row_count.checked_mul(column_count).ok_or_else(|| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Scoreboard] dimensions {rows}x{columns} overflow the host address space"
        ))
    })?;
    let mut cells = Vec::new();
    cells.try_reserve_exact(cell_count).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Scoreboard] dimensions {rows}x{columns} cannot be allocated"
        ))
    })?;
    for row in 0..row_count {
        for column in 0..column_count {
            let string_key = format!("Cell{column}_{row}String");
            let value_key = format!("Cell{column}_{row}Value");
            let text = tree
                .value(section, &string_key)
                .map(decode_legacy_game_string)
                .ok_or_else(|| {
                    ScenarioError::InitialNetworkRuntime(format!(
                        "[Scoreboard] missing required `{string_key}`"
                    ))
                })?;
            let value = tree
                .value(section, &value_key)
                .and_then(parse_std_i32)
                .ok_or_else(|| {
                    ScenarioError::InitialNetworkRuntime(format!(
                        "[Scoreboard] missing or invalid required `{value_key}`"
                    ))
                })?;
            cells.push((Some(text), value));
        }
    }
    ScoreboardState::from_compiled_cells(row_count, column_count, show_count, cells).ok_or_else(
        || {
            ScenarioError::InitialNetworkRuntime(format!(
                "[Scoreboard] dimensions {rows}x{columns} disagree with the compiled cell matrix"
            ))
        },
    )
}

fn effect_var_from_value(value: clonk_script::Value) -> EffectVarValue {
    use clonk_script::Value;
    match value {
        Value::Int(value) => EffectVarValue::Int(value),
        Value::Bool(value) => EffectVarValue::Bool(value),
        Value::RawBool(value) => EffectVarValue::RawBool(value),
        Value::String(value) => EffectVarValue::String(value),
        Value::C4Id(value) => EffectVarValue::C4Id(value),
        Value::Object(value) => EffectVarValue::Object(value),
        Value::Array(values) => {
            EffectVarValue::Array(values.into_iter().map(effect_var_from_value).collect())
        }
        Value::Proplist(values) => EffectVarValue::Proplist(values),
        Value::Nil => EffectVarValue::Nil,
    }
}
