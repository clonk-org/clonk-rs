use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use clonk_resources::definition::{
    ActionFacet as ResourceActionFacet, DefCore as ResourceDefCore,
    DefinitionGraphicsVariant as ResourceGraphicsVariant,
};
use clonk_resources::{
    decode_legacy_script_text, load_image_from_memory, localize_script_source_with_components,
    ActionDefinition as ResourceActionDefinition, ActionMap as ResourceActionMap, ColorByOwnerMask,
    ComponentGroups, DefinitionError as ResourceDefinitionError, GraphicsImage, Group, GroupError,
    LanguagePacks, ParticleDefinition as ResourceParticleDefinition, RankNameTable,
    ResourceDefinition as ResourceDefinitionData,
};
use image::{ImageError, ImageFormat};
use serde::de::Error as _;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::action::is_builtin_idle_name;
use crate::landscape::{
    LandscapeRasterState, RuntimeTexMapLookup, RuntimeTexMapMaterial, RuntimeTexMapState,
};
use crate::network_game_data::{
    decode_legacy_game_string, parse_landscape_game_data, InitialNetworkGameApplyError,
    InitialNetworkGameData, InitialNetworkGameError, LandscapeGameData,
};
use crate::{
    action::ActionSpec, ActionState, CommandDirection, Definition, DefinitionActionFacet,
    DefinitionActionGraphics, DefinitionComponent, DefinitionId, DefinitionPicture,
    DefinitionPictureImage, DefinitionRect, DefinitionSpriteImage, Direction, EffectState,
    EffectVarValue, Engine, EngineError, EnvironmentSettings, Landscape, LegacyCString,
    MaterialSet, MovementProfile, ObjectId, ObjectStatus, PhysicsSettings, RgbColor,
    RoundResultsState, ScoreboardState, ScriptGlobalState, SkyFrame, SkyParallaxMode, SkySettings,
    SpawnConfig, TeamInfo, Vector2, LANDSCAPE_MODE_DYNAMIC, LANDSCAPE_MODE_EXACT,
    LANDSCAPE_MODE_STATIC,
};

mod c4value;
mod core;
mod definitions;
mod legacy_parse;
mod legacy_types;
mod map;
mod sections;
mod values;
pub mod verbose_loading;

pub(crate) use c4value::*;
pub use core::*;
pub(crate) use definitions::*;
pub use legacy_parse::*;
pub use legacy_types::*;
pub use map::*;
pub(crate) use sections::*;
pub use values::*;

#[cfg(test)]
fn write_test_definition_graphics(path: &Path) {
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
        .save(path.join("Graphics.png"))
        .expect("write definition graphics");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::bmp::BmpEncoder, ColorType, Rgba, RgbaImage};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{subscriber, Level};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[track_caller]
    fn test_tempdir() -> tempfile::TempDir {
        tempdir().expect("test directory builds")
    }

    #[track_caller]
    fn write_test_file(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        std::fs::write(path, contents).expect("test file writes");
    }

    fn test_resolver(roots: Vec<PathBuf>) -> FileSystemResolver {
        FileSystemResolver { roots }
    }

    trait TestValueExt<T> {
        fn test_value(self) -> T;
    }

    impl<T> TestValueExt<T> for Option<T> {
        #[track_caller]
        fn test_value(self) -> T {
            self.expect("scenario-test value exists")
        }
    }

    impl<T, E: std::fmt::Debug> TestValueExt<T> for Result<T, E> {
        #[track_caller]
        fn test_value(self) -> T {
            self.expect("scenario-test operation succeeds")
        }
    }

    #[track_caller]
    fn load_test_scenario<R: LegacyDefinitionResolver>(
        path: impl AsRef<Path>,
        resolver: &R,
    ) -> Scenario {
        Scenario::load_from_path_with(path, resolver).expect("test scenario loads")
    }

    #[track_caller]
    fn apply_test_scenario(scenario: &Scenario, engine: &mut Engine) -> Vec<ObjectId> {
        scenario.apply(engine).expect("test scenario applies")
    }

    trait TestEngineExt {
        fn register_test_definition(&mut self, definition: Definition);
        fn register_test_player(&mut self, player: crate::PlayerConfig);
        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
        fn test_object_index(&self, object: ObjectId) -> usize;
    }

    impl TestEngineExt for Engine {
        #[track_caller]
        fn register_test_definition(&mut self, definition: Definition) {
            self.register_definition(definition)
                .expect("definition registers");
        }

        #[track_caller]
        fn register_test_player(&mut self, player: crate::PlayerConfig) {
            self.register_player(player).expect("player registers");
        }

        #[track_caller]
        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
            self.spawn_object(config).expect("object spawns")
        }

        #[track_caller]
        fn test_object_index(&self, object: ObjectId) -> usize {
            self.find_object_index(object).expect("object exists")
        }
    }

    #[test]
    fn initial_network_scenario_matches_pristine_cpp_tutorial01_differential() {
        // C4GameSave::SaveCore + C4GameSaveNetwork::AdjustCore and
        // C4Scenario::CompileFunc (C4GameSave.cpp:58-108,612-617;
        // C4Scenario.cpp:100-134,164-439).
        let source = include_str!("../../../content/Tutorial.c4f/Tutorial01.c4s/Scenario.txt");
        let scenario = scenario_with_retained_legacy_core(source);
        let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];

        let actual = scenario
            .serialize_initial_network_scenario(
                "A Clonk",
                &definitions,
                "",
                "",
                "Tutorial.c4f/Tutorial01.c4s",
            )
            .expect("legacy initial network scenario serializes");

        assert_eq!(
            actual,
            TUTORIAL01_PRISTINE_CPP_INITIAL_NETWORK_SCENARIO.as_bytes()
        );
    }

    #[test]
    fn material_and_texture_names_use_c4m_max_name_bytes() {
        let material_prefix = b"MaterialPrefix\x80";
        let texture_prefix = b"TexturePrefixX\x81";
        assert_eq!(material_prefix.len(), 15);
        assert_eq!(texture_prefix.len(), 15);

        let mut material_long_a = material_prefix.to_vec();
        material_long_a.extend_from_slice(b"First");
        let mut material_long_b = material_prefix.to_vec();
        material_long_b.extend_from_slice(b"Second");
        let mut texture_long_a = texture_prefix.to_vec();
        texture_long_a.extend_from_slice(b"One");
        let mut texture_long_b = texture_prefix.to_vec();
        texture_long_b.extend_from_slice(b"Two");

        let material_source = |name: &[u8], density: i32| {
            let mut source = b"[Material]\nName=".to_vec();
            source.extend_from_slice(name);
            source.extend_from_slice(format!("\nDensity={density}\nTextureOverlay=").as_bytes());
            source.extend_from_slice(texture_prefix);
            source.push(b'\n');
            source
        };
        let mut texmap_source = b"20=".to_vec();
        texmap_source.extend_from_slice(material_prefix);
        texmap_source.push(b'-');
        texmap_source.extend_from_slice(texture_prefix);
        texmap_source.extend_from_slice(b"\n21=");
        texmap_source.extend_from_slice(&material_long_a);
        texmap_source.push(b'-');
        texmap_source.extend_from_slice(&texture_long_a);
        texmap_source.push(b'\n');

        let parsed_texmap = clonk_resources::texmap::TextureMap::parse_bytes(&texmap_source);
        assert_eq!(
            clonk_script::c4_string_bytes(&parsed_texmap.entry(21).unwrap().material),
            material_long_a,
            "TexMap names remain raw and unbounded"
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&parsed_texmap.entry(21).unwrap().texture),
            texture_long_a,
            "non-UTF-8 TexMap bytes survive without replacement"
        );

        let mut materials = clonk_resources::MutableGroup::new("Material.c4g");
        materials
            .add_file("A.c4m", material_source(&material_long_a, 61))
            .unwrap();
        materials
            .add_file("B.c4m", material_source(&material_long_b, 72))
            .unwrap();
        materials.add_file("TexMap.txt", texmap_source).unwrap();
        let texture_bitmap = encode_indexed_bmp(&[&[0u8]]);
        for stem in [&texture_long_a, &texture_long_b] {
            let mut filename = stem.to_vec();
            filename.extend_from_slice(b".bmp");
            materials
                .add_file_bytes_with_metadata(filename, texture_bitmap.clone(), 1, false)
                .unwrap();
        }
        materials
            .add_file(
                "Mislabeled.bmp",
                include_bytes!("../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .unwrap();

        let mut scenario = clonk_resources::MutableGroup::new("ByteNames.c4s");
        scenario
            .add_packed_child_with_metadata(
                "Material.c4g",
                materials.pack_raw().unwrap(),
                0,
                1,
                false,
            )
            .unwrap();
        let group =
            Group::from_raw_memory(PathBuf::from("ByteNames.c4s"), scenario.pack_raw().unwrap())
                .unwrap();
        let classifier =
            build_map_pixel_classifier(&group, &FileSystemResolver { roots: Vec::new() })
                .unwrap()
                .unwrap();
        let library = classifier.material_library().unwrap();
        let names = library
            .iter()
            .map(|material| clonk_script::c4_string_bytes(material.name()))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![material_prefix.to_vec(), material_prefix.to_vec()]
        );
        for material in library.iter() {
            assert_eq!(
                material.value("Name").map(clonk_script::c4_string_bytes),
                Some(material_prefix.to_vec()),
                "compiled Name reflection matches the fixed live core"
            );
        }
        assert_eq!(
            library
                .get(&clonk_script::c4_string_from_bytes(material_prefix))
                .and_then(|material| material.int("Density")),
            Some(61),
            "the first same-load fixed-name collision owns name lookup"
        );
        assert!(
            library
                .get(&clonk_script::c4_string_from_bytes(&material_long_a))
                .is_none(),
            "whole-name lookup does not prefix-match a fixed identity"
        );

        assert_eq!(
            classifier
                .state
                .texture_inventory
                .iter()
                .map(|name| clonk_script::c4_string_bytes(name))
                .collect::<Vec<_>>(),
            vec![texture_prefix.to_vec(), texture_prefix.to_vec()],
            "long candidates admit before truncation, but a PNG payload named BMP is rejected"
        );
        assert_eq!(
            classifier.state.densities[20], 61,
            "the fixed TexMap identity resolves the first material collision"
        );
        assert!(classifier.state.material_names[20].is_some());
        assert!(
            classifier.state.material_names[21].is_none(),
            "an unbounded TexMap pair does not prefix-match fixed identities"
        );
    }

    // Bodies live in byte-verbatim contiguous parts so the module — and
    // every test id it exports — stays exactly as it was.
    include!("scenario/tests/part_01.rs");
    include!("scenario/tests/part_02.rs");
    include!("scenario/tests/part_03.rs");
    include!("scenario/tests/part_04.rs");
    include!("scenario/tests/part_05.rs");
    include!("scenario/tests/part_06.rs");
    include!("scenario/tests/part_07.rs");
    include!("scenario/tests/part_08.rs");
}

#[cfg(test)]
mod game_start_sync {
    use super::*;
    use tempfile::tempdir;

    struct ProbeResolver {
        roots: Vec<std::path::PathBuf>,
    }
    impl LegacyDefinitionResolver for ProbeResolver {
        fn resolve_definition_groups(
            &self,
            scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            let normalized = identifier.replace('\\', "/");
            let path = std::path::Path::new(&normalized);
            let mut groups = Vec::new();
            if let Ok(child) = scenario.open_child(path) {
                groups.push(child);
            }
            for root in &self.roots {
                let candidate = root.join(path);
                if candidate.exists() {
                    groups.push(Group::open(&candidate)?);
                }
            }
            Ok(groups)
        }
    }

    fn write_palm_def(defs: &std::path::Path) {
        let palm = defs.join("Palm.c4d");
        std::fs::create_dir_all(&palm).expect("palm dir");
        std::fs::write(
            palm.join("DefCore.txt"),
            "[DefCore]\nid=PALM\nName=Palm\nCategory=1\nWidth=40\nHeight=56\nOffset=-20,-28\nVertices=1\nVertexY=22\n",
        )
        .expect("defcore");
        std::fs::write(
            palm.join("ActMap.txt"),
            "[Action]\nName=Still\nDelay=4\nLength=1\nNextAction=Still\nStartCall=Still\n\n\
             [Action]\nName=Breeze\nDelay=2\nLength=20\nNextAction=Breeze\nStartCall=Breeze\n",
        )
        .expect("actmap");
        // The real Palm1/Tree StartCalls flip Breeze<->Still by wind
        // (Objects.c4d/Vegetation.c4d): if a loaded spawn fired StartCall,
        // the saved Breeze would collapse to Still like the live bug.
        std::fs::write(
            palm.join("Script.c"),
            "#strict\nfunc Still() { return(1); }\nfunc Breeze() { SetAction(\"Still\"); return(1); }\n",
        )
        .expect("script");
        write_test_definition_graphics(&palm);
    }

    fn load(dir: &std::path::Path) -> (Engine, Scenario) {
        let resolver = ProbeResolver {
            roots: vec![dir.to_path_buf()],
        };
        let scenario_dir = dir.join("Sync.c4s");
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario.apply(&mut engine).expect("scenario applies");
        (engine, scenario)
    }

    fn write_scenario(dir: &std::path::Path, objects: &str) {
        let scenario_dir = dir.join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Sync\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("core");
        std::fs::write(scenario_dir.join("Objects.txt"), objects).expect("objects");
    }

    // C4Game::Init runs SyncClearance + Synchronize AFTER InitGame and
    // BEFORE InitPlayers (C4Game.cpp:474-475): every object's fixed
    // position collapses to itofix(x,y,r) (C4Object::SyncClearance,
    // C4Object.cpp:3803-3815) — grown trees carry y != fixtoi(fix_y) in
    // the savefile because DoCon adjusts y without touching fix_y — and
    // the loaded action restores only when the name resolves in the
    // ActMap (C4Object.cpp:2840-2849); C4Action::Default leaves Name
    // empty, so records without Action= stay ActIdle (no def default:
    // C++ has no such concept). GoldRush oracle: TRE2 #3 (no Action=,
    // FixY 28px below Y) sits at (204,258) Idle in C++; PLM1 #42 keeps
    // Action=Breeze Phase=18. Saved active rows carry Size=FullCon; omitting
    // it compiles Con=0 and correctly forces even a resolved action to Idle.
    #[test]
    fn loaded_objects_sync_clearance_and_action_rules_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(
            dir.path(),
            "[Object]\nid=PALM\nNumber=3\nCategory=1\nSize=100000\nX=204\nY=258\nFixX=f1129054208\nFixY=f1133445120\n\n\
             [Object]\nid=PALM\nNumber=42\nCategory=1\nSize=100000\nX=981\nY=280\nAction=Breeze\nDir=1\nActionTime=694\nPhase=18\nPhaseDelay=4\nYDir=f1149998051\n\n\
             [Object]\nid=PALM\nNumber=43\nCategory=1\nSize=100000\nX=100\nY=100\nAction=Stand\nPhase=2\n",
        );
        let (engine, _) = load(dir.path());

        let (_, action, _phase, position, fix_y, ..) =
            engine.debug_object_by_id(3).expect("tree exists");
        assert_eq!(
            action,
            crate::action::DEFAULT_ACTION_NAME,
            "no Action= -> ActIdle"
        );
        assert_eq!(position, Vector2::new(204, 258), "saved center kept");
        assert_eq!(
            fix_y, 258,
            "SyncClearance collapses fix to itofix(y) (C4Object.cpp:3810)"
        );

        let (_, action, phase, ..) = engine.debug_object_by_id(42).expect("palm exists");
        assert_eq!(action, "Breeze", "resolved saved action survives");
        assert_eq!(phase, 18, "saved phase survives");

        let (_, action, phase, ..) = engine.debug_object_by_id(43).expect("third exists");
        assert_eq!(
            action,
            crate::action::DEFAULT_ACTION_NAME,
            "unresolvable saved action (CCAN Stand) falls to Idle, not a def default"
        );
        assert_eq!(
            phase, 2,
            "failed saved-action lookup leaves the compiled phase untouched"
        );
    }

    #[test]
    fn loaded_actions_restore_signed_counters_and_cpp_data_rules() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("ActMap.txt"),
            "[Action]\nName=Passive\nDelay=0\nLength=1\n\n\
             [Action]\nName=Attached\nProcedure=ATTACH\nDelay=0\nLength=1\n",
        )
        .expect("actmap");
        std::fs::write(
            loaded.join("Script.c"),
            "#strict\npublic func ReadLoadedAction() { return [GetAction(), GetObjectVal(\"Action\"), GetActTime(), GetObjectVal(\"PhaseDelay\"), GetObjectVal(\"ActionData\")]; }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nSize=100000\nX=10\nY=20\nFixX=F999424\nFixY=F1327104\nAction=Passive\nActionTime=-7\nPhase=-2\nPhaseDelay=-3\nActionData=41\n\n\
             [Object]\nid=LOAD\nNumber=101\nCategory=16\nSize=100000\nAction=Attached\nActionTime=-8\nPhase=-4\nPhaseDelay=-5\nActionData=42\n\n\
             [Object]\nid=LOAD\nNumber=102\nCategory=16\nSize=100000\nX=30\nY=40\nFixX=F2015232\nFixY=F2654208\nAction=Missing\nActionTime=-9\nPhase=-6\nPhaseDelay=-7\nActionData=43\n\n\
             [Object]\nid=LOAD\nNumber=103\nCategory=16\nSize=50000\nAction=Attached\nActionTime=-10\nPhase=-8\nPhaseDelay=-9\nActionData=44\n",
        );

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("pre-final-sync scenario applies");
        let passive = engine
            .object_snapshot(ObjectId::new(100))
            .expect("passive object");
        assert_eq!(passive.action.name, "Passive");
        assert_eq!(
            (
                passive.action.time,
                passive.action.phase,
                passive.action.ticks
            ),
            (-7, -2, -3)
        );
        assert_eq!(
            passive.action.data, 41,
            "ActIdle -> DFA_NONE preserves Action.Data"
        );
        let passive_index = engine
            .find_object_index(ObjectId::new(100))
            .expect("passive object is live");
        assert_eq!(
            engine.objects[passive_index].fixed_position,
            crate::math::FixedVec2::from_ints(10, 20),
            "successful load-time SetAction resynchronizes FixX/FixY"
        );

        let attached = engine
            .object_snapshot(ObjectId::new(101))
            .expect("attached object");
        assert_eq!(attached.action.name, "Attached");
        assert_eq!(
            (
                attached.action.time,
                attached.action.phase,
                attached.action.ticks
            ),
            (-8, -4, -5)
        );
        assert_eq!(
            attached.action.data, 0,
            "ActIdle -> non-NONE procedure clears Action.Data"
        );

        let missing = engine
            .object_snapshot(ObjectId::new(102))
            .expect("missing-action object");
        assert_eq!(missing.action.name, crate::action::DEFAULT_ACTION_NAME);
        assert_eq!(missing.action.act_map_index, None);
        assert_eq!(missing.action.raw_name.as_deref(), Some("Missing"));
        assert_eq!(
            (
                missing.action.time,
                missing.action.phase,
                missing.action.ticks
            ),
            (-9, -6, -7)
        );
        assert_eq!(missing.action.data, 43);
        let missing_index = engine
            .find_object_index(ObjectId::new(102))
            .expect("missing-action object is live");
        assert_eq!(
            engine.objects[missing_index].fixed_position,
            crate::math::FixedVec2 {
                x: crate::math::C4Fixed::from_raw(2_015_232),
                y: crate::math::C4Fixed::from_raw(2_654_208),
            },
            "failed SetActionByName retains compiled FixX/FixY"
        );

        let partial = engine
            .object_snapshot(ObjectId::new(103))
            .expect("partial object");
        assert_eq!(partial.action.name, crate::action::DEFAULT_ACTION_NAME);
        assert_eq!(partial.action.compiled_name(), "");
        assert_eq!(
            (
                partial.action.time,
                partial.action.phase,
                partial.action.ticks
            ),
            (-10, -8, -9)
        );
        assert_eq!(
            partial.action.data, 44,
            "incomplete-object coercion remains DFA_NONE -> DFA_NONE"
        );

        assert_eq!(
            engine
                .call_object_function(missing_index, "ReadLoadedAction", Vec::new())
                .expect("raw action probe succeeds"),
            clonk_script::Value::Array(vec![
                clonk_script::Value::String("Idle".to_string().into()),
                clonk_script::Value::String("Missing".to_string().into()),
                clonk_script::Value::Int(-9),
                clonk_script::Value::Int(-7),
                clonk_script::Value::Int(43),
            ])
        );

        let encoded = serde_json::to_string(&missing).expect("snapshot serializes");
        let decoded: crate::ObjectSnapshot =
            serde_json::from_str(&encoded).expect("snapshot deserializes");
        assert_eq!(decoded.action.raw_name.as_deref(), Some("Missing"));
    }

    #[test]
    fn loaded_action_names_truncate_to_the_cpp_30_native_byte_buffer() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");

        // Twenty-eight ASCII bytes plus the two-byte UTF-8 spelling of `é`
        // fill C4Action::Name exactly. The suffix exists only in the source
        // file and must be discarded before SetActionByName runs.
        let matching_name = format!("{}é", "M".repeat(28));
        let unresolved_name = format!("{}é", "U".repeat(28));
        assert_eq!(clonk_script::c4_string_bytes(&matching_name).len(), 30);
        assert_eq!(clonk_script::c4_string_bytes(&unresolved_name).len(), 30);

        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("ActMap.txt"),
            format!("[Action]\nName={matching_name}\nDelay=0\nLength=1\n"),
        )
        .expect("actmap");
        std::fs::write(
            loaded.join("Script.c"),
            "#strict 2\nfunc ReadRawAction() { return GetObjectVal(\"Action\"); }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            &format!(
                "[Object]\nid=LOAD\nNumber=100\nCategory=16\nSize=100000\nX=10\nY=20\nFixX=F999424\nFixY=F1327104\nAction={matching_name}TRAILING\n\n\
                 [Object]\nid=LOAD\nNumber=101\nCategory=16\nSize=100000\nX=30\nY=40\nFixX=F2015232\nFixY=F2654208\nAction={unresolved_name}TRAILING\n"
            ),
        );

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("pre-final-sync scenario applies");

        let matched_index = engine
            .find_object_index(ObjectId::new(100))
            .expect("matched object exists");
        let matched = &engine.objects[matched_index];
        assert_eq!(matched.state.action.name, matching_name);
        assert_eq!(matched.state.action.raw_name, None);
        assert_eq!(
            matched.fixed_position,
            crate::math::FixedVec2::from_ints(10, 20),
            "the truncated physical name resolves and SetAction resynchronizes FixX/FixY"
        );
        assert_eq!(
            engine
                .call_object_function(matched_index, "ReadRawAction", Vec::new())
                .expect("matched raw action reads"),
            clonk_script::Value::String(matching_name.clone().into())
        );

        let unresolved_index = engine
            .find_object_index(ObjectId::new(101))
            .expect("unresolved object exists");
        let unresolved = &engine.objects[unresolved_index];
        assert_eq!(
            unresolved.state.action.name,
            crate::action::DEFAULT_ACTION_NAME
        );
        assert_eq!(
            unresolved.state.action.raw_name.as_deref(),
            Some(unresolved_name.as_str()),
            "a failed lookup retains only the compiled 30-byte buffer"
        );
        assert_eq!(
            unresolved.fixed_position,
            crate::math::FixedVec2 {
                x: crate::math::C4Fixed::from_raw(2_015_232),
                y: crate::math::C4Fixed::from_raw(2_654_208),
            },
            "the unresolved truncated name leaves the serialized fixed position untouched"
        );
        assert_eq!(
            engine
                .call_object_function(unresolved_index, "ReadRawAction", Vec::new())
                .expect("unresolved raw action reads"),
            clonk_script::Value::String(unresolved_name.into())
        );
    }

    #[test]
    fn objects_txt_missing_action_targets_are_null_before_scenario_callbacks() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("Script.c"),
            "#strict\nlocal seen_target1, seen_target2;\npublic func CaptureTargets() { seen_target1 = GetActionTarget(0); seen_target2 = GetActionTarget(1); return 1; }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nAction=Idle\nActionTarget1=999\nActionTarget2=1000\n",
        );
        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nfunc Initialize() { var obj = FindObject(LOAD); obj->CaptureTargets(); return 1; }\n",
        )
        .expect("scenario script");

        let (engine, _) = load(dir.path());
        let object = engine
            .object_snapshot(ObjectId::new(100))
            .expect("loaded object");
        assert_eq!(object.action.target, None);
        assert_eq!(object.action.target2, None);
        assert_eq!(
            object.local_vars.get("seen_target1"),
            Some(&clonk_script::Value::Nil),
            "scenario Initialize runs after ActionTarget1 denumeration"
        );
        assert_eq!(
            object.local_vars.get("seen_target2"),
            Some(&clonk_script::Value::Nil),
            "scenario Initialize runs after ActionTarget2 denumeration"
        );
    }

    #[test]
    fn objects_txt_action_targets_accept_the_old_enumeration_offset() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nActionTarget1=1000000042\nActionTarget2=1000000043\n\n\
             [Object]\nid=LOAD\nNumber=42\nCategory=16\n",
        );

        let (engine, _) = load(dir.path());
        let holder = engine
            .object_snapshot(ObjectId::new(100))
            .expect("holder object");
        assert_eq!(
            holder.action.raw_name.as_deref(),
            Some(""),
            "a missing Action= retains C4Action's empty compiled Name buffer"
        );
        assert_eq!(holder.action.target, Some(ObjectId::new(42)));
        assert_eq!(holder.action.target2, None);
    }

    #[test]
    fn network_apply_defers_cpp_final_sync_until_status_commit() {
        // C4Game::Init performs InitGame before Network.FinalInit; only after
        // every client reaches and acknowledges GO does FinalInit run
        // SyncClearance + Synchronize (pristine 9ffa0a5d src/C4Game.cpp:457-478;
        // src/C4Network2.cpp:558-615).
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(
            dir.path(),
            "[Object]\nid=PALM\nNumber=3\nCategory=1\nX=15\nY=5\nFixX=F999424\nFixY=F327680\n",
        );
        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);

        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("network InitGame phase applies");
        let object = engine
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(3))
            .expect("loaded object exists");
        assert_eq!(
            object.fixed_position.x.val(),
            999_424,
            "network InitGame preserves the saved sub-pixel position before GO commits"
        );

        engine
            .game_start_synchronize()
            .expect("network final synchronization succeeds");
        let object = engine
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(3))
            .expect("loaded object survives final sync");
        assert_eq!(
            object.fixed_position.x.val(),
            crate::math::itofix(15).val(),
            "Network.FinalInit performs the delayed SyncClearance at the GO barrier"
        );
    }

    // C4GameObjects::Load removes inactive rows from the active list before
    // UpdateFaces, so they do not construct or put a solid mask until
    // StatusActivate runs UpdateFace(true). StatusDeactivate does not remove
    // an existing mask.
    #[test]
    fn legacy_loaded_inactive_object_does_not_put_solid_mask_until_activated() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let gate = defs.join("Gate.c4d");
        std::fs::create_dir_all(&gate).expect("gate dir");
        std::fs::write(
            gate.join("DefCore.txt"),
            "[DefCore]\nid=GATE\nName=Gate\nCategory=2\nWidth=1\nHeight=1\nOffset=0,0\nSolidMask=0,0,1,1,0,0\n",
        )
        .expect("defcore");
        std::fs::write(
            gate.join("Script.c"),
            "#strict\npublic func ActivateMask() { return SetObjectStatus(1); }\n\
             public func DeactivateMask() { return SetObjectStatus(2); }\n",
        )
        .expect("script");
        write_test_definition_graphics(&gate);

        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Inactive mask\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("scenario core");
        image::GrayImage::from_pixel(4, 4, image::Luma([0]))
            .save(scenario_dir.join("Landscape.bmp"))
            .expect("landscape");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GATE\nNumber=61\nStatus=2\nCategory=2\nX=10\nY=10\nSize=100000\nWidth=1\nHeight=1\nOffset=0,0\n\n\
             [Object]\nid=GATE\nNumber=62\nStatus=1\nCategory=2\nX=20\nY=10\nSize=100000\nWidth=1\nHeight=1\nOffset=0,0\n",
        )
        .expect("objects");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# dynamic slots only\n").expect("texmap");
        std::fs::write(
            materials.join("Vehicle.c4m"),
            "[Material]\nName=Vehicle\nDensity=100\nTextureOverlay=Smooth\n",
        )
        .expect("vehicle material");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(materials.join("Smooth.png"))
            .expect("texture");

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(61);
        scenario.apply(&mut engine).expect("scenario applies");
        let id = ObjectId::new(61);
        let loaded_normal = ObjectId::new(62);

        assert_eq!(engine.debug_solid_mask_is_put(id.as_u64()), Some(false));
        assert_eq!(engine.debug_landscape_byte(10, 10), Some(0));
        let index = engine.find_object_index(id).expect("inactive gate exists");
        assert_eq!(
            engine.objects[index].solid_mask_instance_sequence, None,
            "the load path must not allocate an ordering slot"
        );
        assert_eq!(
            engine.debug_solid_mask_is_put(loaded_normal.as_u64()),
            Some(true),
            "loaded normal objects still receive the initial UpdateFaces pass"
        );
        assert_eq!(
            engine.debug_landscape_material_name(20, 10).as_deref(),
            Some("Vehicle")
        );

        assert_eq!(
            engine
                .call_object_function(index, "ActivateMask", Vec::new())
                .expect("activation executes"),
            clonk_script::Value::Bool(true)
        );
        assert_eq!(engine.debug_solid_mask_is_put(id.as_u64()), Some(true));
        assert_eq!(
            engine.debug_landscape_material_name(10, 10).as_deref(),
            Some("Vehicle")
        );
        let index = engine.find_object_index(id).expect("active gate exists");
        let activated_sequence = engine.objects[index]
            .solid_mask_instance_sequence
            .expect("activation allocates the mask ordering slot");

        assert_eq!(
            engine
                .call_object_function(index, "DeactivateMask", Vec::new())
                .expect("deactivation executes"),
            clonk_script::Value::Bool(true)
        );
        assert_eq!(
            engine.debug_solid_mask_is_put(id.as_u64()),
            Some(true),
            "runtime deactivation retains the existing mask"
        );
        assert_eq!(
            engine.debug_landscape_material_name(10, 10).as_deref(),
            Some("Vehicle")
        );
        let index = engine.find_object_index(id).expect("inactive gate remains");
        assert_eq!(
            engine.objects[index].solid_mask_instance_sequence,
            Some(activated_sequence),
            "runtime deactivation preserves the existing mask ordering slot"
        );
    }

    // C4Object::CompileFunc reads SolidMask= with DEFAULT Def->SolidMask
    // (C4Object.cpp:2770): a saved 0,0,0,0,0,0 means the object's solid
    // mask is OFF (opened gates/doors save that way); the def's mask must
    // not resurrect it. FnSetSolidMask (C4Script.cpp:271-278) drives the
    // same per-object rect at runtime.
    #[test]
    fn objects_txt_solid_mask_overrides_the_definition_mask() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let gate = defs.join("Gate.c4d");
        std::fs::create_dir_all(&gate).expect("gate dir");
        std::fs::write(
            gate.join("DefCore.txt"),
            "[DefCore]\nid=GATE\nName=Gate\nCategory=2\nWidth=10\nHeight=40\nOffset=-5,-20\nSolidMask=0,0,10,40,0,0\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&gate);
        write_scenario(
            dir.path(),
            "[Object]\nid=GATE\nNumber=7\nCategory=2\nX=50\nY=50\nSolidMask=0,0,0,0,0,0\n\n\
             [Object]\nid=GATE\nNumber=8\nCategory=2\nX=90\nY=50\n",
        );
        let (engine, _) = load(dir.path());

        // The 1x1 fixture bitmap is too small for the def-level 10x40 mask,
        // so like C++ that mask never activates; the loader state is the pin.
        let overrides = engine.debug_solid_mask_override(7);
        assert_eq!(
            overrides,
            Some(Some((0, 0, 0, 0))),
            "saved SolidMask=0,0,0,0,0,0 turns the mask OFF (C4Object.cpp:2770)"
        );
        assert_eq!(
            engine.debug_solid_mask_override(8),
            Some(None),
            "no saved key keeps the definition default"
        );
    }

    // GoldRush DoInitialize (Script.c:33-35) pins unowned crew NPCs from
    // the SCENARIO-SCRIPT context: `while(pObj = FindObjectOwner(0,-1,
    // 0,0,0,0,OCF_CrewMember,0,0,pObj)) AddEffect("StayThere",...)`.
    // The Fx handlers are scenario GLOBALS (Script.c:553-564) — resolved
    // through GetFuncRecursive in C++ (C4Effect.cpp:31-40).
    #[test]
    fn scenario_script_pins_unowned_crew_with_stay_there_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let npc = defs.join("Npc.c4d");
        std::fs::create_dir_all(&npc).expect("npc dir");
        std::fs::write(
            npc.join("DefCore.txt"),
            "[DefCore]\nid=NPCX\nName=Npc\nCategory=66056\nCrewMember=1\nWidth=8\nHeight=20\nOffset=-4,-10\n",
        )
        .expect("npc core");
        write_test_definition_graphics(&npc);

        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Sync\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=NPCX\nNumber=30\nCategory=66056\nX=50\nY=50\nAlive=1\n\n\
             [Object]\nid=NPCX\nNumber=31\nCategory=66056\nX=90\nY=50\nAlive=1\n",
        )
        .expect("objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\n\
             protected func InitializePlayer(iPlr) {\n\
               var i, pObj;\n\
               while(pObj = FindObjectOwner(0,-1,0,0,0,0,OCF_CrewMember,0,0,pObj))\n\
                 AddEffect(\"StayThere\",pObj,1,35,pObj);\n\
               return(1);\n\
             }\n\
             global func FxStayThereStart(pTarget, iNumber, fTmp)\n\
             {\n\
               if(fTmp) return();\n\
               EffectVar(0, pTarget, iNumber) = GetX(pTarget);\n\
               EffectVar(1, pTarget, iNumber) = GetY(pTarget);\n\
             }\n",
        )
        .expect("script");

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(3);
        scenario.apply(&mut engine).expect("scenario applies");
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Test".into(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("join succeeds");

        let snapshot = engine.snapshot();
        let pinned = snapshot
            .objects
            .iter()
            .filter(|object| {
                object.definition_id == "NPCX"
                    && object.effects.iter().any(|e| e.name == "StayThere")
            })
            .count();
        assert_eq!(pinned, 2, "both unowned crew NPCs got StayThere");
        let stored = snapshot
            .objects
            .iter()
            .find(|object| object.id.as_u64() == 30)
            .and_then(|object| {
                object
                    .effects
                    .iter()
                    .find(|e| e.name == "StayThere")
                    .map(|e| e.vars.clone())
            })
            .expect("effect present");
        assert!(
            matches!(stored.first(), Some(crate::effect::EffectVarValue::Int(50))),
            "the GLOBAL FxStayThereStart stored GetX via the seam \
             (C4Effect.cpp:31-40 GetFuncRecursive), got {stored:?}"
        );
    }

    // C4Game::Synchronize's tail broadcasts ~UpdateTransferZone to every
    // active Game.Objects entry (C4Game.cpp:3713-3714,3727-3729;
    // C4GameObjects.cpp:54-58; C4ObjectList.cpp:734-739) AFTER the FixRandom
    // re-fix. GoldRush oracle:
    // the placed cannon's handler
    // (Cannon.c4d/Script.c:20-25) re-runs Initialize() because the stale
    // saved Action=Stand loaded as Idle - SetAction("Ready") +
    // SetDir(Random(2)) (the first draw of the fresh ledger) + the GC4V
    // crosshair as the FIRST created object (C++ NEWOBJ 1420, frame 0,
    // pre-join).
    #[test]
    fn game_start_broadcasts_update_transfer_zone_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let cannon = defs.join("Cannon.c4d");
        std::fs::create_dir_all(&cannon).expect("cannon dir");
        std::fs::write(
            cannon.join("DefCore.txt"),
            "[DefCore]\nid=CANN\nName=Cannon\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&cannon);
        std::fs::write(
            cannon.join("ActMap.txt"),
            "[Action]\nName=Ready\nDelay=0\nLength=8\n",
        )
        .expect("actmap");
        std::fs::write(
            cannon.join("Script.c"),
            "#strict\n\
             protected func Initialize() {\n\
                 SetAction(\"Ready\");\n\
                 SetDir(Random(2));\n\
                 CreateObject(MARK, 0, 0, -1);\n\
                 return(1);\n\
             }\n\
             protected func UpdateTransferZone() { if(ActIdle()) Initialize(); return(1); }\n",
        )
        .expect("script");
        let marker = defs.join("Marker.c4d");
        std::fs::create_dir_all(&marker).expect("marker dir");
        std::fs::write(
            marker.join("DefCore.txt"),
            "[DefCore]\nid=MARK\nName=Marker\nCategory=16\n",
        )
        .expect("marker core");
        write_test_definition_graphics(&marker);

        write_scenario(
            dir.path(),
            "[Object]\nid=CANN\nNumber=439\nCategory=16\nSize=100000\nX=100\nY=100\nAction=Stand\n",
        );
        let (engine, _) = load(dir.path());

        let (_, action, ..) = engine.debug_object_by_id(439).expect("cannon exists");
        assert_eq!(
            action, "Ready",
            "the ~UpdateTransferZone broadcast re-ran Initialize (Cannon.c4d:23)"
        );
        assert!(
            engine.debug_object_by_id(440).is_some(),
            "the crosshair-analog is the FIRST created object (C++ 1420)"
        );

        // The Random(2) draw came off the FRESH post-Synchronize ledger.
        let mut fresh = crate::rng::LcgRng::seed_from_u64(11);
        fresh.random(2);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "SetDir(Random(2)) drew after the FixRandom re-fix (C4Game.cpp:3695,3710)"
        );
    }

    // C4Game::Synchronize re-fixes the RNG AFTER the weather-init draws
    // (C4Game.cpp:3695): the post-apply ledger is a FRESH FixRandom(seed)
    // stream — the join draws from position zero.
    #[test]
    fn game_start_refixes_the_ledger_after_weather_draws_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(dir.path(), "");
        let (engine, _) = load(dir.path());

        let mut fresh = crate::rng::LcgRng::seed_from_u64(11);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "post-apply ledger = fresh FixRandom(seed) (C4Game.cpp:3695)"
        );
    }
}
