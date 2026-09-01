//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

pub(in crate::scenario) fn map_seed_from_random_seed(random_seed: u64) -> i32 {
    let mut rng = crate::rng::LcgRng::seed_from_u64(random_seed);
    rng.random(3_133_700)
}

/// The ChunkOZoom jitter seed: after C4Game::FixRandom(RandomSeed) fills
/// FRndBuf3 with 500 draws, C++ draws `MapSeed = Random(3133700)` before
/// re-fixing for map creation (C4Game.cpp:2651; C4Landscape.cpp:563-579).
/// The shadow bridge can hand the already-drawn C++ value across directly.
fn legacy_map_seed(random_seed: u64) -> i32 {
    let map_seed = std::env::var("LC_RUST_ENGINE_MAP_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or_else(|| map_seed_from_random_seed(legacy_random_seed(random_seed)));
    if std::env::var("LC_DEBUG_MAP").is_ok() {
        eprintln!("RUST MAPSEED {map_seed}");
    }
    map_seed
}

/// `Game.FixRandom(Game.Parameters.RandomSeed)` before map creation
/// (C4Landscape.cpp:578): the map creators draw from a freshly fixed
/// ledger, and the bracket re-fixes afterwards (C4Landscape.cpp:734), so
/// map creation never shifts the post-init synced ledger. The caller supplies
/// the established `Parameters.RandomSeed`; the env shadow remains a test
/// bridge for comparison with the C++ engine.
pub(in crate::scenario) fn legacy_map_creation_rng(random_seed: u64) -> crate::rng::LcgRng {
    legacy_map_creation_rng_traced(random_seed, std::env::var("LC_RUST_RNG_TRACE").is_ok())
}

/// Map creation with the differential probe explicitly armed.
///
/// Native has no separate map generator: `C4Landscape::Init` draws from the one
/// global `Random()`, brackets the whole of map and landscape creation with
/// `Game.FixRandom(Game.Parameters.RandomSeed)` either side
/// (`C4Landscape.cpp:579,735`) so a joining client that does not create the map
/// ends at the same ledger, and traces every one of those draws like any other.
///
/// Modelling the bracket with a separate `LcgRng` is faithful, but leaving it
/// untraced is not: it makes map creation invisible to `LC_RUST_RNG_TRACE`
/// while the oracle records it, so the two traces cannot be compared across the
/// phase where an initialisation divergence would live
/// (clonk-org/clonk-rs#1050).
pub(in crate::scenario) fn legacy_map_creation_rng_traced(
    random_seed: u64,
    trace: bool,
) -> crate::rng::LcgRng {
    crate::rng::LcgRng::seed_from_u64_traced(legacy_random_seed(random_seed), trace)
}

/// C4Game::InitGame overwrites the serialized parameter from the initial
/// player-info list only at frame zero (C4Game.cpp:2455-2456). Runtime
/// records/savegames retain the value captured by C4GameParameters instead.
pub(in crate::scenario) fn replay_startup_player_count_from_group(
    group: &Group,
    serialized_startup_player_count: i32,
) -> Result<i32, ScenarioError> {
    let frame = try_read_group_file_case_insensitive(group, "Game.txt")?
        .map(|source| crate::parse_initial_network_game_data(&source).frame)
        .unwrap_or_default();
    if frame != 0 {
        return Ok(serialized_startup_player_count);
    }

    let Some(source) = try_read_group_file_case_insensitive(group, "PlayerInfos.txt")? else {
        return Ok(0);
    };
    let tree = LegacyIniTree::parse(&bytes_as_latin1_string(&source));
    let Some(root) = tree.first_section(0, "PlayerInfoList") else {
        return Ok(0);
    };
    Ok(tree
        .sections(root, "Client")
        .flat_map(|client| tree.sections(client, "Player"))
        .filter(|player| {
            !tree.value(*player, "Flags").is_some_and(|flags| {
                flags
                    .split(['|', ','])
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .any(|token| {
                        token.eq_ignore_ascii_case("Removed")
                            || parse_std_u32(token).is_some_and(|value| {
                                value & u32::from(crate::PLAYER_INFO_FLAG_REMOVED) != 0
                            })
                    })
            })
        })
        .fold(0_i32, |count, _| count.saturating_add(1)))
}

/// `Game.Parameters.StartupPlayerCount` (MapPlayerExtend input,
/// C4Landscape.cpp:518): the headless harness joins one player.
pub(in crate::scenario) fn legacy_startup_player_count() -> i32 {
    std::env::var("LC_RUST_ENGINE_STARTUP_PLAYERS")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(1)
}

/// The C4MapCreator inputs from the parsed `[Landscape]` section.
fn basic_map_params(landscape: &LegacyLandscape) -> crate::map_creator::BasicMapParams {
    crate::map_creator::BasicMapParams {
        map_width: landscape.map_width,
        map_height: landscape.map_height,
        map_player_extend: landscape.map_player_extend,
        amplitude: landscape.amplitude,
        phase: landscape.phase,
        period: landscape.period,
        random: landscape.random,
        material: landscape.material.clone(),
        liquid: landscape.liquid.clone(),
        liquid_level: landscape.liquid_level,
        layers: landscape
            .layers
            .iter()
            .map(|entry| (entry.name.clone(), entry.count.unwrap_or(0)))
            .collect(),
    }
}

/// Map-pixel material classification (the Pix2Mat/Pix2Dens tables,
/// C4Wrappers.h:110-145, C4Landscape.cpp:2832-2839): a pixel byte's low 7
/// bits are the texmap index (bit 0x80 = IFT); index 0, unmapped entries
/// and unknown materials are sky (MNone, density 0).
#[derive(Clone)]
pub(crate) struct MapPixelClassifier {
    pub(in crate::scenario) state: RuntimeTexMapState,
    material_library: Option<clonk_resources::MaterialLibrary>,
    texmap_lookups: Vec<RuntimeTexMapLookup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaterialTextureLoadCounts {
    /// `None` means C++ found no first material source and therefore emitted
    /// no `LoadMap` count line.
    pub(crate) texmap_entries: Option<usize>,
    pub(crate) textures: usize,
    pub(crate) materials: usize,
}

impl MapPixelClassifier {
    pub(crate) fn from_runtime_state(state: RuntimeTexMapState) -> Self {
        Self {
            state,
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    /// Empty texture-map stand-in used only to reproduce map-creator RNG
    /// consumption when legacy resource activation supplied no classifier.
    fn empty_for_map_creation() -> Self {
        Self {
            state: RuntimeTexMapState {
                densities: vec![0; 128],
                material_names: vec![None; 128],
                texture_names: vec![None; 128],
                match_texture_names: vec![None; 128],
                shapes: vec![None; 128],
                materials: Vec::new(),
                texture_inventory: Vec::new(),
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    pub(crate) fn into_runtime_state(self) -> RuntimeTexMapState {
        self.state
    }

    /// Bare-slot constructor for unit tests (no material groups behind
    /// the slots — `get_index` adds fail like a full C++ texture map).
    #[cfg(test)]
    pub(crate) fn from_slots(
        densities: [i32; 128],
        names: Vec<Option<String>>,
        textures: Vec<Option<String>>,
        shapes: Vec<Option<crate::chunky::ChunkShape>>,
    ) -> Self {
        Self {
            state: RuntimeTexMapState {
                densities: densities.to_vec(),
                material_names: names,
                match_texture_names: textures.clone(),
                texture_names: textures,
                shapes,
                materials: Vec::new(),
                texture_inventory: Vec::new(),
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    /// Test constructor with a material library and texture inventory so
    /// `mat=`/`tex=` validation and `GetIndex` adds behave like a real
    /// scenario load.
    #[cfg(test)]
    pub(crate) fn from_slots_with_library(
        densities: [i32; 128],
        names: Vec<Option<String>>,
        textures: Vec<Option<String>>,
        shapes: Vec<Option<crate::chunky::ChunkShape>>,
        library: clonk_resources::MaterialLibrary,
        texture_inventory: Vec<String>,
    ) -> Self {
        let materials = library
            .iter()
            .map(Self::runtime_material)
            .collect::<Vec<_>>();
        Self {
            state: RuntimeTexMapState {
                densities: densities.to_vec(),
                material_names: names,
                match_texture_names: textures.clone(),
                texture_names: textures,
                shapes,
                materials,
                texture_inventory,
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: Some(library),
            texmap_lookups: Vec::new(),
        }
    }

    pub(in crate::scenario) fn material_library(
        &self,
    ) -> Option<&clonk_resources::MaterialLibrary> {
        self.material_library.as_ref()
    }

    fn runtime_material(material: &clonk_resources::MaterialDefinition) -> RuntimeTexMapMaterial {
        RuntimeTexMapMaterial {
            name: material.name().to_string(),
            density: material.int("Density").unwrap_or(0),
            shape: crate::chunky::ChunkShape::from_shape(material.int("Shape").unwrap_or(0)),
        }
    }

    /// C4TextureMap::CheckTexture (the map creators validate `tex=`
    /// fields against the loaded texture inventory).
    pub(crate) fn texture_exists(&self, name: &str) -> bool {
        self.state.texture_exists(name)
    }

    /// The material definition behind a name, scenario-local first
    /// (C4MaterialMap::Get order after the prepending loads).
    pub(crate) fn material(&self, name: &str) -> Option<&RuntimeTexMapMaterial> {
        self.state.material(name)
    }

    /// C4TextureMap::GetIndex (C4Texture.cpp:319-345): the existing
    /// (material, texture) slot — material match, texture match when
    /// given — else the first free slot when `add_if_not_exist`. 0 = fail.
    pub(crate) fn get_index(
        &mut self,
        mat_name: &str,
        tex_name: Option<&str>,
        add_if_not_exist: bool,
    ) -> u8 {
        self.state.get_index(mat_name, tex_name, add_if_not_exist)
    }

    /// C4TextureMap::GetIndexMatTex (C4Texture.cpp:346-367): split the
    /// `Material-Texture` pair, try the exact pair, then the default
    /// texture; final fallback is the material's default entry
    /// (DefaultMatTex — the first slot carrying the material).
    pub(crate) fn get_index_mat_tex(
        &mut self,
        material_texture: &str,
        default_texture: Option<&str>,
    ) -> u8 {
        let eager_index = self
            .state
            .get_index_mat_tex(material_texture, default_texture);
        self.texmap_lookups.push(RuntimeTexMapLookup {
            material_texture: material_texture.to_owned(),
            default_texture: default_texture.map(str::to_owned),
            eager_index,
        });
        eager_index
    }

    pub(in crate::scenario) fn clear_texmap_lookups(&mut self) {
        self.texmap_lookups.clear();
    }

    pub(in crate::scenario) fn texmap_lookups(&self) -> &[RuntimeTexMapLookup] {
        &self.texmap_lookups
    }
}

/// TexMap.txt + material densities from the ordered NRT_Material chain.
/// C4Game::InitMaterialTexture loads the scenario group first, then each
/// external source admitted by the preceding TexMap's independent
/// OverloadMaterials/OverloadTextures flags (C4Game.cpp:901-977).
/// `None` only when there is no material source at all. A first source
/// without TexMap.txt still builds from an empty table before
/// CrossMapMaterials allocates its dynamic entries.
fn parse_material_enumeration(
    source: Option<&[u8]>,
) -> Result<Option<clonk_resources::material::MaterialEnumeration>, ScenarioError> {
    match source {
        Some(source) if !source.is_empty() => Ok(Some(
            clonk_resources::material::MaterialEnumeration::parse(source)?,
        )),
        Some(_) | None => Ok(None),
    }
}

pub(crate) fn build_map_pixel_classifier(
    group: &Group,
    resolver: &dyn LegacyDefinitionResolver,
) -> Result<Option<MapPixelClassifier>, ScenarioError> {
    build_map_pixel_classifier_with_loaded_counts(group, resolver, |_| {})
}

/// Builds the classifier while publishing the three counts C4Game logs before
/// it attempts `Material.LoadEnumeration` (`C4Game.cpp:940-987`). The callback
/// intentionally runs before a bad `MatMap.txt` can reject the load.
pub(crate) fn build_map_pixel_classifier_with_loaded_counts(
    group: &Group,
    resolver: &dyn LegacyDefinitionResolver,
    mut report_loaded_counts: impl FnMut(MaterialTextureLoadCounts),
) -> Result<Option<MapPixelClassifier>, ScenarioError> {
    // Read the root savegame ledger now, but parse it only after the material
    // counts have been published: C++ calls LoadEnumeration after those logs.
    let enumeration_source = try_read_group_file_case_insensitive(group, "MatMap.txt")?;
    let mut material_groups = Vec::new();
    let mut scenario_material_root = None;
    match group.open_child("Material.c4g") {
        Ok(local) => {
            scenario_material_root = Some(local.root().to_path_buf());
            material_groups.push(local);
        }
        Err(
            GroupError::EntryNotFound(_) | GroupError::Missing(_) | GroupError::NotDirectory(_),
        ) => {}
        Err(error) => return Err(ScenarioError::Resources(error)),
    }
    for candidate in resolver.resolve_material_groups(group)? {
        if scenario_material_root.as_deref() != Some(candidate.root()) {
            material_groups.push(candidate);
        }
    }

    let Some(first_group) = material_groups.first() else {
        report_loaded_counts(MaterialTextureLoadCounts {
            texmap_entries: None,
            textures: 0,
            materials: 0,
        });
        let enumeration = parse_material_enumeration(enumeration_source.as_deref())?;
        if let Some(name) = enumeration
            .as_ref()
            .and_then(|enumeration| enumeration.names().first())
        {
            return Err(
                clonk_resources::material::MaterialEnumerationError::MissingMaterial(name.clone())
                    .into(),
            );
        }
        return Ok(None);
    };
    let texmap = first_group
        .read_file("TexMap.txt")
        .ok()
        .map(|source| clonk_resources::texmap::TextureMap::parse_bytes(&source));

    let mut material_libraries: Vec<clonk_resources::MaterialLibrary> = Vec::new();
    let mut texture_inventory: Vec<String> = Vec::new();
    let mut load_materials = true;
    let mut load_textures = true;
    for (index, material_group) in material_groups.iter().enumerate() {
        if !load_materials && !load_textures {
            break;
        }

        // The first source supplies the actual table. Later sources only
        // expose continuation flags through LoadFlags; a missing TexMap at
        // that point stops both chains before that group's contents load
        // (C4Game.cpp:940-976).
        let later_texmap = if index == 0 {
            None
        } else {
            let Ok(source) = material_group.read_file("TexMap.txt") else {
                break;
            };
            Some(clonk_resources::texmap::TextureMap::parse_flags_bytes(
                &source,
            ))
        };
        let flags = later_texmap.as_ref().or(texmap.as_ref());
        let mut next_materials = flags.is_some_and(|flags| flags.overload_materials);
        let mut next_textures = flags.is_some_and(|flags| flags.overload_textures);

        if load_materials {
            match clonk_resources::MaterialLibrary::from_group(material_group) {
                Ok(library) => {
                    // C4MaterialMap::Load counts only names not provided by an
                    // earlier source. A zero-count load automatically admits
                    // the next source even without OverloadMaterials.
                    let loaded_count = library
                        .iter()
                        .filter(|definition| {
                            material_libraries
                                .iter()
                                .all(|loaded| loaded.get(definition.name()).is_none())
                        })
                        .count();
                    if loaded_count == 0 {
                        next_materials = true;
                    }
                    material_libraries.push(library);
                }
                Err(_) => next_materials = true,
            }
        }

        if load_textures {
            // C4TextureMap::LoadTextures likewise counts only newly admitted
            // image basenames; a zero-count load keeps the texture chain open
            // (C4Texture.cpp:266-310; C4Game.cpp:956-962).
            let mut loaded_count = 0;
            let entries = material_group.entries().unwrap_or_default();
            // LoadTextures scans the whole group twice: PNG first, then BMP.
            for extension in [b".png".as_slice(), b".bmp".as_slice()] {
                for entry in &entries {
                    if entry.is_directory
                        || entry.name_bytes.len() < extension.len()
                        || !entry.name_bytes[entry.name_bytes.len() - extension.len()..]
                            .eq_ignore_ascii_case(extension)
                    {
                        continue;
                    }
                    // SReplaceChar(texname, '.', 0) exposes the bytes before
                    // the first dot, rather than Path::file_stem's suffix.
                    let stem_end = entry
                        .name_bytes
                        .iter()
                        .position(|byte| *byte == b'.')
                        .unwrap_or(entry.name_bytes.len());
                    let full_stem =
                        clonk_script::c4_string_from_bytes(&entry.name_bytes[..stem_end]);
                    // Duplicate detection precedes the fixed-name copy. A
                    // long candidate therefore never equals a stored 15-byte
                    // prefix and every long prefix collision is admitted.
                    if texture_inventory
                        .iter()
                        .any(|stored| clonk_resources::material::c4_names_equal(stored, &full_stem))
                    {
                        continue;
                    }
                    // GroupReadSurfacePNG returns an allocated surface even
                    // if its decoder reports failure. Bitmap admission does
                    // require a successfully decoded Surface8.
                    if extension.eq_ignore_ascii_case(b".bmp")
                        && material_group
                            .read_entry_bytes_exact(entry)
                            .ok()
                            .and_then(|bytes| {
                                clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok()
                            })
                            .is_none()
                    {
                        continue;
                    }
                    texture_inventory
                        .push(clonk_resources::material::truncate_c4m_name(&full_stem));
                    loaded_count += 1;
                }
            }
            if loaded_count == 0 {
                next_textures = true;
            }
        }

        load_materials = next_materials;
        load_textures = next_textures;
    }

    // Each C4MaterialMap::Load prepends its fresh names, so later/global
    // uniques precede earlier/local definitions while earlier sources win
    // collisions (C4Material.cpp:263-299).
    let material_loads: Vec<_> = material_libraries.iter().collect();
    let mut material_library =
        clonk_resources::MaterialLibrary::from_overloaded_loads(&material_loads).ok();
    report_loaded_counts(MaterialTextureLoadCounts {
        texmap_entries: Some(
            texmap
                .as_ref()
                .map(clonk_resources::texmap::TextureMap::loaded_entry_count)
                .unwrap_or_default(),
        ),
        textures: texture_inventory.len(),
        materials: material_library
            .as_ref()
            .map(|library| library.iter().count())
            .unwrap_or_default(),
    });
    let enumeration = parse_material_enumeration(enumeration_source.as_deref())?;

    // Savegames retain the numeric material order in root MatMap.txt. C++
    // applies this pairwise-swap ledger after every material source has
    // loaded but before TextureMap.Init and CrossMapMaterials
    // (C4Game.cpp:979-993; C4Material.cpp:510-558).
    if let Some(enumeration) = enumeration
        .as_ref()
        .filter(|enumeration| !enumeration.is_empty())
    {
        let library = material_library.as_mut().ok_or_else(|| {
            clonk_resources::material::MaterialEnumerationError::MissingMaterial(
                enumeration.names()[0].clone(),
            )
        })?;
        library.sort_enumeration(enumeration)?;
    }

    // LoadMap returns zero for a missing first TexMap, but C++ retains the
    // empty C4TextureMap and still runs Init + CrossMapMaterials. Parsing an
    // empty source gives the normal 128-slot table with both overload flags
    // false, without changing the independent resource-chain decisions above.
    let texmap = texmap.unwrap_or_else(|| clonk_resources::texmap::TextureMap::parse_bytes(b""));
    let overload_materials = texmap.overload_materials;
    let overload_textures = texmap.overload_textures;

    let runtime_materials = material_library
        .iter()
        .flat_map(|library| library.iter())
        .map(MapPixelClassifier::runtime_material)
        .collect();

    let mut densities = [0i32; 128];
    let mut names: Vec<Option<String>> = vec![None; 128];
    let mut grid_textures: Vec<Option<String>> = vec![None; 128];
    let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
    for (index, slot) in densities.iter_mut().enumerate() {
        names[index] = texmap
            .entry(index as u8)
            .map(|entry| entry.material.clone());
        grid_textures[index] = texmap.entry(index as u8).map(|entry| entry.texture.clone());
        let material = texmap.entry(index as u8).and_then(|entry| {
            material_library
                .as_ref()
                .and_then(|library| library.get(&entry.material))
        });
        shapes[index] = material.map(|material| {
            crate::chunky::ChunkShape::from_shape(material.int("Shape").unwrap_or(0))
        });
        *slot = material
            .and_then(|material| material.int("Density"))
            .unwrap_or(0);
        // "Special, hardcoded crap": liquids render <mat>-Smooth with
        // the Liquid texture (C4TexMapEntry::Init, C4Texture.cpp:79-82).
        if (25..50).contains(&*slot)
            && grid_textures[index]
                .as_deref()
                .is_some_and(|texture| clonk_resources::material::c4_names_equal(texture, "Smooth"))
        {
            grid_textures[index] = Some("Liquid".to_string());
        }
    }
    // Raw texmap textures for GetIndex pair matching.
    let mut match_textures: Vec<Option<String>> = vec![None; 128];
    for (index, slot) in match_textures.iter_mut().enumerate() {
        *slot = texmap.entry(index as u8).map(|entry| entry.texture.clone());
    }
    // Collected as owned (name, overlay, cross-specs) rows so the loops below
    // can mutate the classifier slots.
    let ordered: Vec<(String, Option<String>, Vec<String>)> = material_library
        .iter()
        .flat_map(|library| library.iter())
        .map(|material| {
            (
                material.name().to_string(),
                material.value("TextureOverlay").map(str::to_string),
                ["BlastShiftTo", "BelowTempConvertTo", "AboveTempConvertTo"]
                    .iter()
                    .filter_map(|key| material.strings(key).first().cloned())
                    .filter(|spec| !spec.is_empty())
                    .collect(),
            )
        })
        .collect();

    let mut classifier = MapPixelClassifier {
        state: RuntimeTexMapState {
            densities: densities.to_vec(),
            material_names: names,
            texture_names: grid_textures,
            shapes,
            match_texture_names: match_textures,
            materials: runtime_materials,
            texture_inventory,
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            overload_materials,
            overload_textures,
            ..Default::default()
        },
        material_library,
        texmap_lookups: Vec::new(),
    };

    // C4TextureMap::Init initializes every parsed entry only after the final
    // material and texture inventories have loaded. Entries whose material
    // or effective texture cannot be resolved are cleared before
    // CrossMapMaterials, making their slots available to the ascending
    // GetIndex allocation scan (C4Texture.cpp:68-104,229-244). For liquid
    // `Material-Smooth` entries, `texture_names` already carries the
    // hard-coded effective `Liquid` lookup while `match_texture_names`
    // deliberately retains the raw `Smooth` pair.
    let invalid_slots = (1..127usize)
        .filter(|&slot| {
            let Some(material_name) = classifier.state.material_names[slot].as_deref() else {
                return false;
            };
            classifier.state.material(material_name).is_none()
                || classifier.state.texture_names[slot]
                    .as_deref()
                    .is_none_or(|texture_name| !classifier.state.texture_exists(texture_name))
        })
        .map(|slot| slot as u8)
        .collect::<Vec<_>>();
    classifier.state.clear_entries(&invalid_slots);

    // Dynamic texmap entries (C4MaterialMap::CrossMapMaterials,
    // C4Material.cpp:345-484): the DefaultMatTex loop registers
    // (MaterialName, TextureOverlay-or-"Smooth") for EVERY material with
    // fAddIfNotExist — an exact (mat, tex) pair miss fills the FIRST FREE
    // slot (1, 2, 3, …) — then the BlastShiftTo/BelowTempConvertTo/
    // AboveTempConvertTo specs go through GetIndexMatTex the same way.
    // Legacy maps rely on the deterministic slots: GoldRush's road pixels
    // are byte 3 = the third add, Vehicle-Smooth (live-probe-verified
    // slots: [Ice-Sponge, FlyAshes-Spots, Vehicle-Smooth, Ashes-Spots,
    // Ore-Structure, Tunnel-Smooth2, Brick-Brick, Rock2-Rough]).
    // First loop: DefaultMatTex (C4Material.cpp:349-370).
    for (name, overlay, _) in &ordered {
        if name.is_empty() {
            continue;
        }
        let overlay = overlay
            .as_deref()
            .filter(|overlay| classifier.texture_exists(overlay))
            .unwrap_or("Smooth")
            .to_string();
        let default_entry = classifier.get_index(name, Some(&overlay), true);
        classifier
            .state
            .set_default_material_entry(name, default_entry);
    }
    // Second loop: the cross-ref specs (C4Material.cpp:474-484).
    for (_, _, specs) in &ordered {
        for spec in specs {
            let entry = classifier.get_index_mat_tex(spec, None);
            classifier.state.material_crossmap_entries.push(entry);
        }
    }

    if std::env::var("LC_DEBUG_MAP").is_ok() {
        for slot in 1..9usize {
            eprintln!(
                "RUSTTEX {slot} = {:?} density={}",
                classifier.state.material_names[slot], classifier.state.densities[slot]
            );
        }
    }
    Ok(Some(classifier))
}

fn invalid_exact_landscape_pixel(width: u32, slot: usize, byte: u8) -> ScenarioError {
    let width = width.max(1) as usize;
    let x = slot % width;
    let y = slot / width;
    ScenarioError::InvalidLandscape(format!(
        "landscape loading error at ({x}/{y}): pixel value {byte} is not a valid material"
    ))
}

/// Convert the two historical exact-landscape byte formats and enforce the
/// current PixCol2Mat gate. The two native branches are deliberately not one
/// match: a PNG entry suppresses format-0 conversion, but format 1 converts
/// independently and never goes through the live-byte validation afterwards
/// (C4Landscape.cpp:1557-1600).
fn convert_exact_landscape_indices(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    texmap: &RuntimeTexMapState,
    format: i32,
    png_present: bool,
) -> Result<Vec<u8>, ScenarioError> {
    let mut indices = bitmap.indices.clone();
    let material_count = texmap.materials.len();

    if !png_present && format == 0 {
        for (slot, byte) in indices.iter_mut().enumerate() {
            let source = *byte;
            let old_index = usize::from(source & 63);
            let material = (source >= 128 && old_index < material_count.saturating_mul(3))
                .then_some(old_index / 3)
                .ok_or_else(|| invalid_exact_landscape_pixel(bitmap.width, slot, source))?;
            // Native Mat2PixColDefault(MNone) indexes outside the material
            // array for malformed format-0 input. Reject that undefined case
            // rather than manufacturing a material byte.
            let default = texmap
                .default_material_entry_by_index(material as i32)
                .unwrap_or(0);
            let ift = if source >= 192 { 0x80 } else { 0 };
            *byte = default.wrapping_add(ift);
        }
    }

    if format == 1 {
        let vehicle = texmap
            .materials
            .iter()
            .position(|material| {
                clonk_resources::material::c4_names_equal(&material.name, "Vehicle")
            })
            .map_or(-1, |index| index as i32);
        let material_count = material_count as i32;
        for byte in &mut indices {
            let source = *byte;
            let mut material = i32::from(source & 0x7f) - 1;
            if material > vehicle {
                if material == vehicle + 1 {
                    material = vehicle;
                } else {
                    material -= 2;
                }
            }
            *byte = if (0..material_count).contains(&material) {
                texmap
                    .default_material_entry_by_index(material)
                    .unwrap_or(0)
                    .wrapping_add(source & 0x80)
            } else {
                0
            };
        }
        return Ok(indices);
    }

    for (slot, &byte) in indices.iter().enumerate() {
        if byte == 0 {
            continue;
        }
        let texmap_slot = usize::from(byte & 0x7f);
        let valid = (1..127).contains(&texmap_slot)
            && texmap
                .material_names
                .get(texmap_slot)
                .and_then(Option::as_ref)
                .is_some();
        if !valid {
            return Err(invalid_exact_landscape_pixel(bitmap.width, slot, byte));
        }
    }
    Ok(indices)
}

fn decode_exact_landscape_png(source: &[u8], width: u32, height: u32) -> Result<Vec<u32>, String> {
    let rgba = image::load_from_memory_with_format(source, ImageFormat::Png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if rgba.width() < width || rgba.height() < height {
        // Native code performs unchecked source reads for a smaller PNG.
        // Contain that undefined case as the same nonfatal PNG-load failure.
        return Err(format!(
            "Landscape.png is {}x{}, smaller than Landscape.bmp {width}x{height}",
            rgba.width(),
            rgba.height()
        ));
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
            let transparency = 255_u8.wrapping_sub(alpha);
            let color = (u32::from(transparency) << 24)
                | (u32::from(red) << 16)
                | (u32::from(green) << 8)
                | u32::from(blue);
            pixels.push(if transparency == 0xff {
                0xff00_0000
            } else {
                color
            });
        }
    }
    Ok(pixels)
}

/// Install an exact landscape's decoded index plane directly as Surface8.
/// C4Landscape::Load keeps the texture map but no C4Landscape::Map, and does
/// not apply MapZoom/ChunkOZoom (C4Landscape.cpp:658-668,1520-1600).
pub(in crate::scenario) fn exact_classified_landscape(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    classifier: &MapPixelClassifier,
    map_seed: i32,
    format: i32,
    landscape_png: Option<&[u8]>,
) -> Result<Landscape, ScenarioError> {
    let surface32_pixels = landscape_png.and_then(|source| {
        match decode_exact_landscape_png(source, bitmap.width, bitmap.height) {
            Ok(pixels) => Some(pixels),
            Err(error) => {
                tracing::error!(
                    error,
                    "could not load 32-bit landscape surface from Landscape.png"
                );
                None
            }
        }
    });
    let indices = convert_exact_landscape_indices(
        bitmap,
        &classifier.state,
        format,
        landscape_png.is_some(),
    )?;
    let world_height = bitmap.height as i32;
    let mut landscape = Landscape::new(bitmap.width, vec![world_height; bitmap.width as usize])
        .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
    landscape.set_world_height(world_height);
    let mut pixels = crate::landscape::PixelGrid::new(
        bitmap.width,
        bitmap.height,
        indices,
        classifier.state.densities.clone(),
        classifier.state.material_names.clone(),
        classifier.state.texture_names.clone(),
    );
    if let Some(surface32_pixels) = surface32_pixels {
        pixels.install_initial_surface32_pixels(surface32_pixels);
    }
    landscape.set_pixel_grid(pixels);
    landscape.refresh_all_raster_columns();
    landscape.set_raster_state(LandscapeRasterState::new(
        0,
        map_seed,
        classifier.state.clone(),
    ));
    Ok(landscape)
}

/// Build the landscape from a classified 8-bit map: the map zooms through
/// ChunkOZoom into the Surface8 pixel plane (chunky material rims and
/// slope smoothers, C4Landscape::MapToSurface → TexOZoom → ChunkOZoom,
/// C4Landscape.cpp:336-480), then the column approximation — surface
/// heights, liquid segments, IFT tunnel ranges — derives from that plane.
pub(crate) fn classified_landscape(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    classifier: &MapPixelClassifier,
    zoom: i32,
    map_seed: i32,
) -> Result<Landscape, ScenarioError> {
    let map_width = bitmap.width as i32;
    let map_height = bitmap.height as i32;
    let rendered_width = bitmap.width.saturating_mul(zoom as u32);
    let rendered_height = map_height.saturating_mul(zoom).max(0) as u32;
    // C4Landscape::Init clamps the allocated Surface8 independently of the
    // map's zoomed render rectangle (C4Landscape.cpp:638-641). MapToSurface
    // still clips ChunkOZoom to the smaller rectangle, so pad its finished
    // bytes instead of letting inclusive Flat/rough chunk edges bleed into
    // the right/bottom sky margin.
    let final_width = rendered_width.max(100);
    let final_height = rendered_height.max(100);
    let world_height = final_height as i32;
    let plane_width = final_width as usize;

    let synthesized = crate::chunky::synthesize_landscape(
        &bitmap.indices,
        map_width,
        map_height,
        zoom,
        map_seed,
        &classifier.state.shapes,
    )
    .into_bytes();
    let bytes = if (final_width, final_height) == (rendered_width, rendered_height) {
        synthesized
    } else {
        let final_width = final_width as usize;
        let rendered_width = rendered_width as usize;
        let rendered_height = rendered_height as usize;
        let mut padded = vec![0; final_width * final_height as usize];
        for row in 0..rendered_height {
            let source = row * rendered_width;
            let target = row * final_width;
            padded[target..target + rendered_width]
                .copy_from_slice(&synthesized[source..source + rendered_width]);
        }
        padded
    };
    let mut landscape = Landscape::new(final_width, vec![world_height; plane_width])
        .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
    landscape.set_world_height(world_height);
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        final_width,
        final_height,
        bytes,
        classifier.state.densities.clone(),
        classifier.state.material_names.clone(),
        classifier.state.texture_names.clone(),
    ));
    landscape.refresh_all_raster_columns();
    let mut raster_state = LandscapeRasterState::new(zoom, map_seed, classifier.state.clone());
    raster_state.set_map(bitmap);
    landscape.set_raster_state(raster_state);

    // Loaded water is at rest: C4MassMoverSet starts empty and movers are
    // created only by landscape CHANGES via CheckInstability, never at
    // load (C4Game.cpp:1782 MassMover.Default(); the c4b Load only fires
    // for saved games).
    Ok(landscape)
}

pub(in crate::scenario) fn load_legacy_landscape(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
) -> Result<Option<Landscape>, ScenarioError> {
    let mut ignore_progress = |_: i32, _: &str| {};
    load_legacy_landscape_with_progress(
        group,
        manifest,
        runtime,
        overload_current,
        classifier,
        random_seed,
        startup_player_count,
        map_callback_functions,
        post_init_map_callbacks,
        prepared_map_creator,
        &mut ignore_progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::scenario) fn load_legacy_landscape_with_progress(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
    report_progress: &mut dyn FnMut(i32, &str),
) -> Result<Option<Landscape>, ScenarioError> {
    *post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
    let Some(mut landscape) = load_legacy_landscape_body_with_progress(
        group,
        manifest,
        runtime,
        overload_current,
        classifier,
        random_seed,
        startup_player_count,
        map_callback_functions,
        post_init_map_callbacks,
        prepared_map_creator,
        report_progress,
    )?
    else {
        return Ok(None);
    };
    landscape.set_shade_materials(manifest.core.landscape.shade_materials);
    // C4Landscape::Init captures pInitial before attempting to load the
    // optional legacy diff. ApplyDiff failure is non-fatal, including a
    // missing or unreadable DiffLandscape.bmp.
    let diff = if let Some(bytes) = try_read_group_file_case_insensitive(group, "DiffLandscape.bmp")
        .ok()
        .flatten()
    {
        clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok()
    } else {
        None
    };
    if landscape.pixel_grid().is_some() {
        landscape
            .save_initial()
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        if let Some(diff) = diff.as_ref() {
            let _ = landscape.apply_diff(diff);
        }
    } else if diff.is_some() {
        return Err(ScenarioError::InvalidLandscape(
            "DiffLandscape.bmp requires a Surface8 pixel grid".to_string(),
        ));
    }
    // C4Landscape::ScenarioInit (C4Landscape.cpp:67-73): the border-open
    // keys, then the AutoScanSideOpen side scan over the built landscape.
    let borders = &manifest.core.landscape;
    landscape.set_no_scan(borders.no_scan);
    landscape.set_border_open(
        borders.left_open,
        borders.right_open,
        borders.top_open,
        borders.bottom_open,
    );
    if borders.auto_scan_side_open {
        landscape.scan_side_open();
    }
    Ok(Some(landscape))
}

pub(in crate::scenario) fn load_legacy_landscape_body(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
) -> Result<Option<Landscape>, ScenarioError> {
    let mut ignore_progress = |_: i32, _: &str| {};
    load_legacy_landscape_body_with_progress(
        group,
        manifest,
        runtime,
        overload_current,
        classifier,
        random_seed,
        startup_player_count,
        map_callback_functions,
        post_init_map_callbacks,
        prepared_map_creator,
        &mut ignore_progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::scenario) fn load_legacy_landscape_body_with_progress(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
    report_progress: &mut dyn FnMut(i32, &str),
) -> Result<Option<Landscape>, ScenarioError> {
    *prepared_map_creator = None;
    let landscape_section = manifest.sections.get("landscape");
    let map_width_hint = manifest.core.landscape.map_width.std.max(1);
    let map_height_hint = manifest.core.landscape.map_height.std.max(1);
    let exact_landscape = manifest.core.landscape.exact_landscape;
    let map_seed = runtime
        .map(|runtime| runtime.map_seed)
        .filter(|seed| *seed != 0)
        .unwrap_or_else(|| legacy_map_seed(random_seed));
    let precompiled_mode = runtime
        .map(|runtime| runtime.mode)
        .filter(|mode| *mode != 0);
    let set_initial_mode = |landscape: &mut Landscape, inferred| {
        if let Some(mode) = precompiled_mode {
            landscape.set_runtime_mode(mode);
        } else {
            let _ = landscape.set_mode(inferred);
        }
    };
    let mut map_rng = legacy_map_creation_rng(random_seed);

    let read_optional = |name: &str| {
        try_read_group_file_case_insensitive(group, name).map_err(ScenarioError::Resources)
    };

    // ExactLandscape: Landscape.bmp IS the landscape — C++ reads it
    // straight into the pixel surface (GroupReadSurface8), so it decodes
    // at pixel scale (zoom 1) here. Returning no landscape would leave
    // GBackSolid answering "never solid" and hang placement loops in real
    // content (Grass.c4d Initialize).
    let (map_bytes, map_zoom_override, old_landscape_map) = if exact_landscape {
        // C4Landscape::Load requires C4CFN_Landscape. Exact mode never falls
        // back to Map.bmp (C4Landscape.cpp:1520-1524).
        (
            Some(read_group_file_case_insensitive(group, "Landscape.bmp")?),
            Some(1),
            false,
        )
    } else {
        // Static map: Map.bmp, with Landscape.bmp accepted as the map for
        // downwards compatibility (C4Landscape.cpp:593-601) — most CR
        // content (GoldRush included) ships only Landscape.bmp.
        match read_optional("Map.bmp")? {
            Some(bytes) => (Some(bytes), None, false),
            None => {
                let fallback = read_optional("Landscape.bmp")?;
                let map_changed = fallback.is_some();
                (fallback, None, map_changed)
            }
        }
    };
    let exact_landscape_png = if exact_landscape {
        read_optional("Landscape.png")?
    } else {
        None
    };

    let mut classifier = classifier;
    if let Some(bytes) = map_bytes {
        let retained_indexed =
            clonk_resources::bitmap::IndexedBitmap::decode_with_palette(&bytes).ok();
        let retained_indexed_map = retained_indexed.as_ref().map(|(bitmap, _)| bitmap.clone());
        // Material-classified path: the map's 8-bit palette indices are
        // texmap keys (GroupReadSurface8 keeps the index bytes). Without
        // a TexMap or for non-indexed images, the sky-pixel heuristic
        // below stands in.
        if let Some(classifier) = classifier.take() {
            if let Some((bitmap, source_palette)) = retained_indexed.as_ref() {
                let mut landscape = if exact_landscape {
                    let landscape = exact_classified_landscape(
                        bitmap,
                        classifier,
                        map_seed,
                        manifest.core.landscape.new_style_landscape,
                        exact_landscape_png.as_deref(),
                    )?;
                    // C4Landscape::Load validates the exact Surface8 before
                    // its 70 checkpoint; Init reaches 80 only after Load
                    // returns successfully (C4Landscape.cpp:1520-1608,658-674).
                    report_progress(70, "Landscape source map prepared");
                    report_progress(80, "Landscape pixel maps prepared");
                    landscape
                } else {
                    report_progress(70, "Landscape source map prepared");
                    report_progress(80, "Landscape pixel maps prepared");
                    let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
                    classified_landscape(bitmap, classifier, map_zoom_u32 as i32, map_seed)?
                };
                set_initial_mode(
                    &mut landscape,
                    if exact_landscape {
                        LANDSCAPE_MODE_EXACT
                    } else {
                        LANDSCAPE_MODE_STATIC
                    },
                );
                if exact_landscape {
                    landscape
                        .raster_state_mut()
                        .expect("exact classified landscapes carry raster state")
                        .set_surface_palette(*source_palette);
                }
                if old_landscape_map {
                    landscape
                        .raster_state_mut()
                        .expect("classified landscapes carry raster state")
                        .set_map_changed();
                }
                report_progress(87, "Landscape raster constructed");
                return Ok(Some(landscape));
            }
        }
        let dynamic = load_image_from_memory(&bytes)
            .map_err(|source| ScenarioError::LegacyMapDecode { source })?;
        let rgba = dynamic.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        if width == 0 || height == 0 {
            return Err(ScenarioError::LegacyMapEmpty);
        }
        report_progress(70, "Landscape source map prepared");

        let map_zoom_u32 =
            map_zoom_override.unwrap_or_else(|| legacy_map_zoom(landscape_section, &mut map_rng));
        let map_zoom_i32 = map_zoom_u32 as i32;
        let sky_pixel = rgba.get_pixel(0, 0).0;
        let rendered_height = (height as i32).saturating_mul(map_zoom_i32).max(0);
        let world_height = if exact_landscape {
            rendered_height
        } else {
            rendered_height.max(100)
        };
        let capacity = (width as usize).saturating_mul(map_zoom_u32 as usize);
        let mut surfaces = Vec::with_capacity(capacity);

        for x in 0..width {
            // The landscape column model stores the SURFACE Y coordinate
            // (solid from `y >= surface`): the first non-sky map row, zoomed.
            // An all-sky column has no solid (surface at the world bottom).
            let surface_world = (0..height)
                .find(|&y| rgba.get_pixel(x, y).0 != sky_pixel)
                .map(|y| (y as i32).saturating_mul(map_zoom_i32))
                .unwrap_or(world_height);

            for _ in 0..map_zoom_u32 {
                surfaces.push(surface_world);
            }
        }

        if surfaces.iter().all(|&surface| surface >= world_height) {
            // No solid anywhere (the whole map read as sky): fall back to a
            // ground level from the hints so the world has a floor.
            let ground_height = manifest
                .ground_height_hint
                .map(|hint| hint.max(0))
                .unwrap_or(0);
            let surface = (world_height - ground_height).max(0);
            surfaces.fill(surface);
        }

        report_progress(80, "Landscape pixel maps prepared");
        let rendered_width = width.saturating_mul(map_zoom_u32);
        let final_width = if exact_landscape {
            rendered_width
        } else {
            rendered_width.max(100)
        };
        surfaces.resize(final_width as usize, world_height);
        let mut landscape = Landscape::new(final_width, surfaces)
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        // GBackHgt is known exactly here (map height × zoom); placement
        // searches and `Random(GBackHgt - 32)` draws bound on it.
        landscape.set_world_height(world_height);
        set_initial_mode(
            &mut landscape,
            if exact_landscape {
                LANDSCAPE_MODE_EXACT
            } else {
                LANDSCAPE_MODE_STATIC
            },
        );
        let mut raster_state = LandscapeRasterState::new(
            if exact_landscape { 0 } else { map_zoom_i32 },
            map_seed,
            RuntimeTexMapState::default(),
        );
        if exact_landscape {
            if let Some((_, source_palette)) = retained_indexed.as_ref() {
                raster_state.set_surface_palette(*source_palette);
            }
        }
        if !exact_landscape {
            if let Some(bitmap) = retained_indexed_map.as_ref() {
                raster_state.set_map(bitmap);
            }
        }
        if old_landscape_map {
            raster_state.set_map_changed();
        }
        landscape.set_raster_state(raster_state);
        report_progress(87, "Landscape raster constructed");
        return Ok(Some(landscape));
    }

    if exact_landscape {
        return Ok(None);
    }

    let landscape_script = read_optional("Landscape.txt")?;
    if overload_current && landscape_script.is_none() {
        // Section Init is an overload: unlike initial game creation it does
        // not fall back to C4MapCreator::CreateMap. The current Surface8 is
        // retained, PXS/MassMover state stays live, and LandscapeLoaded stays
        // false.
        return Ok(None);
    }

    // Dynamic map (C4Landscape::Init, C4Landscape.cpp:606-614): a
    // Landscape.txt map description renders through C4MapCreatorS2
    // (CreateMapS2, C4Landscape.cpp:530-546); otherwise the basic
    // C4MapCreator builds the 8-bit map from the [Landscape] keys. Both
    // draw from the FixRandom(RandomSeed) bracket (C4Landscape.cpp:
    // 578,734), so they never shift the post-init synced ledger.
    // Requires a texture map for the material bytes.
    if let Some(classifier) = classifier.take() {
        let players = startup_player_count;
        let landscape_core = &manifest.core.landscape;
        let mut retained_creator = None;
        let bitmap = if let Some(bytes) = landscape_script.as_deref() {
            let creation = crate::map_creator_s2::create_s2_map_with_state_and_functions(
                &String::from_utf8_lossy(bytes),
                classifier,
                landscape_core.map_width,
                landscape_core.map_height,
                landscape_core.map_player_extend,
                players,
                &mut map_rng,
                map_callback_functions,
            );
            *post_init_map_callbacks = creation.callbacks;
            // CreateMapS2 keeps pMapCreator alive through InitializeDef,
            // placements and PostInitMap even when KeepMapCreator is false;
            // PostInitMap performs the conditional destruction afterward.
            retained_creator = Some(creation.creator);
            *prepared_map_creator = retained_creator.clone();
            match creation.bitmap {
                Some(bitmap) => bitmap,
                None if overload_current => return Ok(None),
                None => {
                    // Dynamic map by scenario (C4Landscape.cpp:612-614) is
                    // available only during initial, non-overload creation.
                    let params = basic_map_params(landscape_core);
                    crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
                }
            }
        } else {
            let params = basic_map_params(landscape_core);
            crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
        };
        report_progress(70, "Landscape source map prepared");
        let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
        post_init_map_callbacks.set_map_zoom(map_zoom_u32 as i32);
        if let Some(creator) = retained_creator.as_mut() {
            creator.set_callback_map_zoom(map_zoom_u32 as i32);
        }
        *prepared_map_creator = retained_creator.clone();
        report_progress(80, "Landscape pixel maps prepared");
        let mut landscape =
            classified_landscape(&bitmap, classifier, map_zoom_u32 as i32, map_seed)?;
        landscape
            .raster_state_mut()
            .expect("classified landscapes carry raster state")
            .set_map_creator(retained_creator);
        set_initial_mode(&mut landscape, LANDSCAPE_MODE_DYNAMIC);
        report_progress(87, "Landscape raster constructed");
        return Ok(Some(landscape));
    }

    // Even without activated material resources, C++ has already run the
    // map creator before evaluating MapZoom. Render into an empty classifier
    // solely to advance this local FixRandom ledger to the same position;
    // the flat compatibility landscape below remains the returned result.
    let players = startup_player_count;
    let landscape_core = &manifest.core.landscape;
    let mut discarded_classifier = MapPixelClassifier::empty_for_map_creation();
    let mut discarded_creator = None;
    if let Some(bytes) = landscape_script.as_deref() {
        let creation = crate::map_creator_s2::create_s2_map_with_state_and_functions(
            &String::from_utf8_lossy(bytes),
            &mut discarded_classifier,
            landscape_core.map_width,
            landscape_core.map_height,
            landscape_core.map_player_extend,
            players,
            &mut map_rng,
            map_callback_functions,
        );
        *post_init_map_callbacks = creation.callbacks;
        discarded_creator = Some(creation.creator);
        *prepared_map_creator = discarded_creator.clone();
        if creation.bitmap.is_none() {
            if overload_current {
                return Ok(None);
            }
            let params = basic_map_params(landscape_core);
            let _ = crate::map_creator::create_basic_map(
                &params,
                &mut discarded_classifier,
                players,
                &mut map_rng,
            );
        }
    } else {
        let params = basic_map_params(landscape_core);
        let _ = crate::map_creator::create_basic_map(
            &params,
            &mut discarded_classifier,
            players,
            &mut map_rng,
        );
    }

    report_progress(70, "Landscape source map prepared");
    let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
    post_init_map_callbacks.set_map_zoom(map_zoom_u32 as i32);
    if let Some(creator) = discarded_creator.as_mut() {
        creator.set_callback_map_zoom(map_zoom_u32 as i32);
    }
    *prepared_map_creator = discarded_creator.clone();
    let width_product = i64::from(map_width_hint).saturating_mul(i64::from(map_zoom_u32));
    let width_u32 = width_product
        .clamp(1, i64::from(u32::MAX))
        .try_into()
        .unwrap_or(u32::MAX)
        .max(100);
    let fallback_height = map_height_hint.saturating_mul(map_zoom_u32 as i32).max(100);
    report_progress(80, "Landscape pixel maps prepared");
    let mut landscape = Landscape::flat(width_u32, fallback_height);
    landscape.set_world_height(fallback_height);
    if discarded_creator.is_some() {
        let mut raster_state =
            LandscapeRasterState::new(map_zoom_u32 as i32, map_seed, RuntimeTexMapState::default());
        raster_state.set_map_creator(discarded_creator);
        landscape.set_raster_state(raster_state);
    }
    set_initial_mode(&mut landscape, LANDSCAPE_MODE_DYNAMIC);
    report_progress(87, "Landscape raster constructed");
    Ok(Some(landscape))
}

pub(in crate::scenario) fn parse_legacy_c4s_value(
    _field: &str,
    raw: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    let mut values = [defaults.std, defaults.rnd, defaults.min, defaults.max];
    // C4SVal::CompileFunc defaults its individual members independently of
    // the outer naming adaptor's prefilled scenario-specific value.
    compile_defaulted_i32_components(raw, &mut values, &[0, 0, 0, 100], false);
    Ok(LegacyC4SVal::new(
        values[0], values[1], values[2], values[3],
    ))
}

fn legacy_c4s_value(
    entries: Option<&Vec<(String, String)>>,
    key: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    match entries.and_then(|entries| find_entry_including_empty(entries, key)) {
        Some(raw) => parse_legacy_c4s_value(key, raw, defaults),
        None => Ok(defaults),
    }
}

/// C4Surface::LoadAny extension search order for extension-less names
/// (C4Surface.cpp:855).
const LEGACY_SKY_EXTENSIONS: [&str; 4] = ["png", "bmp", "jpeg", "jpg"];

/// The default sky fade when `SkyDefFade` has a signed sum of zero: game
/// palette entries CSkyDef1=104 and 104+19 (C4Sky::SetFadePalette,
/// C4Sky.cpp:56-62; C4Landscape.h:34), scaled `<< 2` at load
/// (C4GraphicsResource.cpp:183-184). Values read from
/// planet/Graphics.c4g/C4.PAL.
const LEGACY_SKY_FADE_TOP_DEFAULT: RgbColor = RgbColor::new(28, 64, 152);
const LEGACY_SKY_FADE_BOTTOM_DEFAULT: RgbColor = RgbColor::new(192, 196, 252);

/// Mirrors C4Sky::Init for legacy scenario loads (C4Sky.cpp:71-152): first
/// try the scenario's implicit `Sky` bitmap, then pick one entry from SkyDef
/// with stateless SeededRandom and search the scenario before Graphics.c4g
/// (C4Sky.cpp:82-105). A loaded bitmap gets white fade, is tiled up to
/// 128x128 (SurfaceEnsureSize, C4Sky.cpp:28-52,109-111), and applies the
/// SkyScrollMode parallax mapping (C4Sky.cpp:114-125). Without one the sky is
/// the `SkyDefFade` gradient (SetFadePalette, C4Sky.cpp:54-68).
pub(in crate::scenario) fn derive_legacy_sky(
    group: &Group,
    resolver: &dyn LegacyDefinitionResolver,
    definition_roots: &[Group],
    manifest: &mut LegacyScenarioManifest,
    random_seed: u64,
) -> Result<SkyConfig, ScenarioError> {
    let mut settings = SkySettings::default();
    let mut surface = load_legacy_sky_surface(group, "Sky");

    if surface.is_none() {
        // C4Sky::Init mutates the stored SkyDef before section selection, so
        // scripts observe semicolons even when the selected bitmap loads.
        if let Some(sky_def) = manifest.core.landscape.sky.as_mut() {
            *sky_def = sky_def.replace(',', ";");
        }

        let sky_def = manifest.core.landscape.sky.as_deref().unwrap_or_default();
        // split() preserves leading, consecutive, and trailing empty slots;
        // C++ counts all of them with SCharCount(';') + 1.
        let section_count = sky_def.split(';').count();
        let selected_index =
            crate::rng::LcgRng::seeded_random(random_seed as u32, section_count as u32) as usize;
        let selected = sky_def
            .split(';')
            .nth(selected_index)
            .unwrap_or_default()
            .trim()
            .to_string();

        if !selected.is_empty() && selected != "Default" {
            surface = load_legacy_sky_surface(group, &selected);
            if surface.is_none() {
                let graphics_groups = resolver
                    .resolve_graphics_groups_with_definition_roots(group, definition_roots)?;
                surface = load_legacy_sky_surface_from_groups(&graphics_groups, &selected);
            }
        }
    }

    if let Some((width, height, pixels)) = surface {
        settings.fade_top = RgbColor::new(255, 255, 255);
        settings.fade_bottom = RgbColor::new(255, 255, 255);
        // SkyScrollMode (C4Sky.cpp:114-125): 1 = wind-driven xdir with
        // stronger y-parallax; 2 = stronger parallax both ways
        // (ParallaxMode itself stays Fixed in case 2, like C++).
        match manifest.core.landscape.sky_scroll_mode {
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
        let (width, height, pixels) = ensure_sky_surface_size(width, height, pixels, 128, 128);
        settings = settings.with_surface(width, height);
        return Ok(SkyConfig {
            settings,
            surface: Some(Arc::new(GraphicsImage::new(width, height, pixels))),
        });
    }

    // No sky surface: fade gradient (C4Sky.cpp:129-134). A zero signed sum
    // across SkyDefFade selects the palette default (C4Sky.cpp:56-62).
    let fade = manifest.core.landscape.sky_fade;
    if fade.iter().sum::<i32>() == 0 {
        settings.fade_top = LEGACY_SKY_FADE_TOP_DEFAULT;
        settings.fade_bottom = LEGACY_SKY_FADE_BOTTOM_DEFAULT;
    } else {
        // C4RGB projects every signed channel through `& 0xff`; it does not
        // clamp values outside the byte range (StdColors.h:52).
        let channel = |value: i32| value as u8;
        settings.fade_top = RgbColor::new(channel(fade[0]), channel(fade[1]), channel(fade[2]));
        settings.fade_bottom = RgbColor::new(channel(fade[3]), channel(fade[4]), channel(fade[5]));
    }
    Ok(SkyConfig {
        settings,
        surface: None,
    })
}

/// C4Surface::LoadAny filename candidates. An explicit extension suppresses
/// extension probing; otherwise png/bmp/jpeg/jpg are tried in this order
/// (C4Surface.cpp:846-865).
fn legacy_sky_filename_patterns(name: &str) -> Vec<String> {
    if Path::new(name)
        .extension()
        .is_some_and(|extension| !extension.is_empty())
    {
        vec![name.to_string()]
    } else {
        LEGACY_SKY_EXTENSIONS
            .iter()
            .map(|extension| format!("{name}.{extension}"))
            .collect()
    }
}

/// Byte-for-byte equivalent of StdFile's ASCII-insensitive `WildcardMatch`,
/// used by C4Group::FindEntry for SkyDef names such as `Pyroclastic*`.
pub(in crate::scenario) fn legacy_group_wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            let next_value = saved_value + 1;
            pattern_index = saved_pattern;
            value_index = next_value;
            backtrack_value = Some(next_value);
        } else {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

enum LegacySkyEntryMatch {
    Missing,
    Found(Option<Vec<u8>>),
}

fn read_legacy_group_wildcard(group: &Group, pattern: &str) -> LegacySkyEntryMatch {
    let Ok(entries) = group.entries() else {
        return LegacySkyEntryMatch::Missing;
    };
    let Some(entry) = entries
        .into_iter()
        .find(|entry| legacy_group_wildcard_match(pattern.as_bytes(), entry.name_bytes.as_slice()))
    else {
        return LegacySkyEntryMatch::Missing;
    };
    LegacySkyEntryMatch::Found(group.read_entry_bytes_exact(&entry).ok())
}

fn decode_legacy_sky_surface(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    load_image_from_memory(bytes).ok().map(|decoded| {
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        (width, height, rgba.into_raw())
    })
}

/// Load a named surface from one scenario group. The first existing extension
/// is decoded once; a broken higher-priority file does not fall through to a
/// lower-priority extension (C4Surface.cpp:846-865).
pub(in crate::scenario) fn load_legacy_sky_surface(
    group: &Group,
    name: &str,
) -> Option<(u32, u32, Vec<u8>)> {
    for pattern in legacy_sky_filename_patterns(name) {
        match read_legacy_group_wildcard(group, &pattern) {
            LegacySkyEntryMatch::Missing => {}
            LegacySkyEntryMatch::Found(bytes) => {
                return bytes.as_deref().and_then(decode_legacy_sky_surface);
            }
        }
    }
    None
}

/// Load an extensionless SkyDef name from the ordered GraphicsResource group
/// set. C4Surface's group-set overload searches extension before group and,
/// due to its exact control flow, returns false for already-extended names
/// (C4Surface.cpp:867-890).
fn load_legacy_sky_surface_from_groups(
    groups: &[Group],
    name: &str,
) -> Option<(u32, u32, Vec<u8>)> {
    if Path::new(name)
        .extension()
        .is_some_and(|extension| !extension.is_empty())
    {
        return None;
    }
    for extension in LEGACY_SKY_EXTENSIONS {
        let pattern = format!("{name}.{extension}");
        for group in groups {
            match read_legacy_group_wildcard(group, &pattern) {
                LegacySkyEntryMatch::Missing => {}
                LegacySkyEntryMatch::Found(bytes) => {
                    return bytes.as_deref().and_then(decode_legacy_sky_surface);
                }
            }
        }
    }
    None
}

/// SurfaceEnsureSize (C4Sky.cpp:28-52): enlarge to at least
/// `min_width` x `min_height` by whole-tile repetition of the original.
fn ensure_sky_surface_size(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    min_width: u32,
    min_height: u32,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 {
        return (width, height, pixels);
    }
    let mut dest_width = width;
    let mut dest_height = height;
    while dest_width < min_width {
        dest_width += width;
    }
    while dest_height < min_height {
        dest_height += height;
    }
    if dest_width == width && dest_height == height {
        return (width, height, pixels);
    }
    let row_bytes = (width * 4) as usize;
    let mut enlarged = Vec::with_capacity((dest_width * dest_height * 4) as usize);
    for y in 0..dest_height {
        let source_row = &pixels[(y % height) as usize * row_bytes..][..row_bytes];
        for _ in 0..dest_width / width {
            enlarged.extend_from_slice(source_row);
        }
    }
    (dest_width, dest_height, enlarged)
}

pub(in crate::scenario) fn derive_legacy_physics(
    manifest: &LegacyScenarioManifest,
) -> Result<(Option<PhysicsSettings>, LegacyC4SVal), ScenarioError> {
    let gravity_defaults = LegacyC4SVal::new(100, 0, 10, 200);
    let entries = manifest.sections.get("landscape");
    if entries.is_none() {
        return Ok((None, gravity_defaults));
    }
    let gravity = legacy_c4s_value(entries, "gravity", gravity_defaults)?;
    let mut physics = PhysicsSettings::default();
    physics.gravity = gravity.base();
    Ok((Some(physics), gravity))
}

/// The C4SVals C4Weather::Init evaluates at scenario start
/// (C4Weather.cpp:36-70) plus the NoInitialize gate for the rain-cloud
/// block (:49-58) and the late NoGamma assignment (:65).
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct LegacyWeatherInit {
    #[doc(hidden)]
    pub season: LegacyC4SVal,
    #[doc(hidden)]
    pub year_speed: LegacyC4SVal,
    #[doc(hidden)]
    pub climate: LegacyC4SVal,
    #[doc(hidden)]
    pub wind: LegacyC4SVal,
    #[doc(hidden)]
    pub rain: LegacyC4SVal,
    #[doc(hidden)]
    pub precipitation: String,
    #[doc(hidden)]
    pub lightning: LegacyC4SVal,
    #[doc(hidden)]
    pub meteorite: LegacyC4SVal,
    #[doc(hidden)]
    pub volcano: LegacyC4SVal,
    #[doc(hidden)]
    pub earthquake: LegacyC4SVal,
    #[doc(hidden)]
    pub no_initialize: bool,
    #[doc(hidden)]
    pub no_gamma: bool,
}

pub(in crate::scenario) fn derive_legacy_weather_init(
    manifest: &LegacyScenarioManifest,
) -> Result<LegacyWeatherInit, ScenarioError> {
    let weather = manifest.sections.get("weather");
    let disasters = manifest.sections.get("disasters");
    // C4SWeather::Default (C4Scenario.cpp:372-379) and
    // C4SDisasters::Default (:427-432); C4SVal::Default = (0,0,0,100).
    Ok(LegacyWeatherInit {
        season: legacy_c4s_value(weather, "startseason", LegacyC4SVal::new(50, 50, 0, 100))?,
        year_speed: legacy_c4s_value(weather, "yearspeed", LegacyC4SVal::new(50, 0, 0, 100))?,
        climate: legacy_c4s_value(weather, "climate", LegacyC4SVal::new(50, 10, 0, 100))?,
        wind: legacy_c4s_value(weather, "wind", LegacyC4SVal::new(0, 70, -100, 100))?,
        rain: legacy_c4s_value(weather, "rain", LegacyC4SVal::new(0, 0, 0, 100))?,
        precipitation: manifest.core.weather.precipitation.clone(),
        lightning: legacy_c4s_value(weather, "lightning", LegacyC4SVal::new(0, 0, 0, 100))?,
        meteorite: legacy_c4s_value(disasters, "meteorite", LegacyC4SVal::new(0, 0, 0, 100))?,
        volcano: legacy_c4s_value(disasters, "volcano", LegacyC4SVal::new(0, 0, 0, 100))?,
        earthquake: legacy_c4s_value(disasters, "earthquake", LegacyC4SVal::new(0, 0, 0, 100))?,
        no_initialize: manifest.core.head.no_initialize != 0,
        no_gamma: manifest.core.weather.no_gamma,
    })
}

pub(in crate::scenario) fn derive_legacy_environment(
    manifest: &LegacyScenarioManifest,
) -> Result<EnvironmentSettings, ScenarioError> {
    let weather_entries = manifest.sections.get("weather");
    let disasters_entries = manifest.sections.get("disasters");

    let wind_defaults = LegacyC4SVal::new(0, 70, -100, 100);
    let wind = legacy_c4s_value(weather_entries, "wind", wind_defaults)?;
    let mut environment = EnvironmentSettings::new(wind.base());
    environment.set_legacy_wind_value(wind.std, wind.rnd, wind.min, wind.max);

    let climate_defaults = LegacyC4SVal::new(50, 10, 0, 100);
    let climate_value = legacy_c4s_value(weather_entries, "climate", climate_defaults)?;
    let climate = 100 - climate_value.base() - 50;
    environment = environment.with_climate(climate);
    environment = environment.with_temperature(climate);

    let season_defaults = LegacyC4SVal::new(50, 50, 0, 100);
    let season_value = legacy_c4s_value(weather_entries, "startseason", season_defaults)?;
    // C4Weather::Init assigns StartSeason.Evaluate() without an extra
    // clamp (C4Weather.cpp:41); the C4SVal Min/Max also bound Execute's
    // season wrap (:82-83).
    environment.season = season_value.base();
    environment = environment.with_season_bounds(season_value.min, season_value.max);

    let year_defaults = LegacyC4SVal::new(50, 0, 0, 100);
    let year_speed = legacy_c4s_value(weather_entries, "yearspeed", year_defaults)?.base();
    environment = environment.with_year_speed(year_speed);

    let rain_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let rain_value = legacy_c4s_value(weather_entries, "rain", rain_defaults)?.base();
    environment = environment.with_precipitation(rain_value);
    environment = environment.with_precipitation_strength(rain_value);

    let lightning_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let lightning = legacy_c4s_value(weather_entries, "lightning", lightning_defaults)?.base();
    environment = environment.with_lightning(lightning);

    let no_gamma = weather_entries
        .and_then(|entries| find_entry(entries, "nogamma"))
        .and_then(|value| parse_legacy_bool(&value))
        .unwrap_or(true);
    environment = if no_gamma {
        environment.with_gamma_disabled()
    } else {
        environment.with_gamma_enabled()
    };

    let disaster_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let meteorite = legacy_c4s_value(disasters_entries, "meteorite", disaster_defaults)?.base();
    let volcano = legacy_c4s_value(disasters_entries, "volcano", disaster_defaults)?.base();
    let earthquake = legacy_c4s_value(disasters_entries, "earthquake", disaster_defaults)?.base();
    environment = environment
        .with_meteorite(meteorite)
        .with_volcano(volcano)
        .with_earthquake(earthquake);

    Ok(environment)
}

/// `C4ScenSect_Main` (src/C4Scenario.cpp:555).
pub(in crate::scenario) const SCENARIO_SECTION_MAIN: &str = "main";

/// `C4ScenarioSection`'s constructor folds an empty name, and any case of
/// `main`, onto `C4ScenSect_Main` (src/C4Scenario.cpp:555-566):
///
/// ```cpp
/// C4ScenarioSection::C4ScenarioSection(char *szName)
///     : Name{(szName && !SEqualNoCase(szName, C4ScenSect_Main) && *szName)
///         ? szName : C4ScenSect_Main}
/// ```
///
/// Every other section keeps its authored case. This is not cosmetic:
/// `C4GameSave::SaveScenarioSections` composes each written entry name from
/// the stored name (src/C4GameSave.cpp:111-137), so a scenario that ships
/// `SectMain.c4g` is saved back as `Sectmain.c4g`. C4Group lookups are
/// case-insensitive, so a reload finds it either way — but the saved bytes
/// differ, which matters wherever an entry name reaches a hash or a byte
/// comparison.
pub(in crate::scenario) fn folded_scenario_section_name(name: &str) -> String {
    if name.is_empty() || name.eq_ignore_ascii_case(SCENARIO_SECTION_MAIN) {
        SCENARIO_SECTION_MAIN.to_string()
    } else {
        name.to_string()
    }
}

pub(in crate::scenario) fn legacy_scenario_section_name(
    path: &Path,
) -> Result<Option<String>, ScenarioError> {
    if path.components().count() != 1 {
        return Ok(None);
    }
    let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
        return Ok(None);
    };
    let lower = filename.to_ascii_lowercase();
    if !lower.starts_with("sect") || !lower.ends_with(".c4g") {
        return Ok(None);
    }
    let name = &filename[4..filename.len() - 4];
    // C4Game::LoadScenarioSections rejects an empty or over-long name *before*
    // constructing the section (src/C4Game.cpp:3318-3322), so the constructor's
    // empty-name fold is unreachable from discovery.
    if name.is_empty() || name.len() > 30 {
        return Err(ScenarioError::InvalidScenarioSectionName {
            path: path.to_path_buf(),
        });
    }
    Ok(Some(folded_scenario_section_name(name)))
}

pub(in crate::scenario) fn load_legacy_landscape_systems(
    group: &Group,
) -> Result<ScenarioLandscapeSystems, ScenarioError> {
    let mut ignore_progress = |_: i32, _: &str| {};
    load_legacy_landscape_systems_with_progress(group, &mut ignore_progress)
}

pub(in crate::scenario) fn load_legacy_landscape_systems_with_progress(
    group: &Group,
    report_progress: &mut dyn FnMut(i32, &str),
) -> Result<ScenarioLandscapeSystems, ScenarioError> {
    let pxs = read_optional_legacy_entry(group, "PXS.c4b")?
        .map(|bytes| {
            crate::pxs::PxsSystem::from_c4b(&bytes)
                .map_err(|error| ScenarioError::LegacyParse(error.to_string()))
        })
        .transpose()?;
    report_progress(91, "PXS loading complete");
    let mass_movers = read_optional_legacy_entry(group, "MassMover.c4b")?
        .map(|bytes| {
            crate::mass_mover::MassMoverSet::from_c4b(&bytes)
                .map_err(|error| ScenarioError::LegacyParse(error.to_string()))
        })
        .transpose()?;
    report_progress(92, "Mass mover loading complete");
    Ok(ScenarioLandscapeSystems { pxs, mass_movers })
}
