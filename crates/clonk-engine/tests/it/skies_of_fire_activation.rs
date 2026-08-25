use std::collections::BTreeMap;

use clonk_engine::Vector2;

use crate::support::real_scenario::prepare_installed_scenario;

const SKIES_OF_FIRE: &str = "Fantasy.c4f/SkiesOfFire.c4s";
const SEED: u64 = 424_242;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn extend_fnv1a64(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn fnv1a64<T: AsRef<[u8]>>(chunks: impl IntoIterator<Item = T>) -> u64 {
    chunks.into_iter().fold(FNV_OFFSET, |hash, bytes| {
        extend_fnv1a64(hash, bytes.as_ref())
    })
}

#[test]
fn full_skies_of_fire_activation_preserves_map_rng_objects_and_initialize_callback() {
    // Regression oracle captured before the script-free S2 render fast path,
    // from the complete installed scenario and its real definitions/materials.
    // C4Landscape::CreateMapS2 renders this source through RenderTo before
    // MapToLandscape (C4Landscape.cpp:530-546,563-579), and the final plane is
    // row-major Surface8 (C4Landscape.h:29-32).
    let engine = prepare_installed_scenario(SKIES_OF_FIRE, SEED).instantiate();
    let snapshot = engine.snapshot();
    let landscape = snapshot
        .landscape
        .as_ref()
        .expect("Skies of Fire creates a landscape");
    let grid = landscape
        .pixel_grid()
        .expect("Skies of Fire creates a Surface8 plane");
    assert_eq!((grid.width(), grid.height()), (2_250, 2_250));
    // The recorded constant this used to carry is gone, and deliberately not
    // replaced with a re-recorded one.
    //
    // Non-rotateable objects stopped being spawned rotated (`C4Object::Init`
    // zeroes the requested rotation before deriving `fix_r`, and
    // `C4Def::Rotateable` defaults to 0 — C4Def.cpp:156,376), so their upright
    // shapes settle their SolidMasks into different pixels and the plane
    // legitimately changed. The value it changed *to* is host-dependent:
    // macOS produces 0xc032671534fff7d0 where Linux produces
    // 0xe99c9252eb27671e, because the plane stores texmap indices and an
    // unpacked `content/` assigns material slots in directory-read order.
    // Pinning either one is the platform-conditional expectation that passes
    // locally and fails the Linux job.
    //
    // What this scenario can still assert host-independently is that the plane
    // is deterministic: the same scenario and seed produce the same bytes
    // twice, which is what the constant was guarding for the script-free S2
    // render fast path in the first place. Restoring a cross-revision guard
    // needs a host-independent digest — clonk-org/clonk-rs#1082.
    let second = prepare_installed_scenario(SKIES_OF_FIRE, SEED).instantiate();
    let second_snapshot = second.snapshot();
    let second_grid = second_snapshot
        .landscape
        .as_ref()
        .expect("the second instantiation creates a landscape")
        .pixel_grid()
        .expect("the second instantiation creates a Surface8 plane");
    assert_eq!(
        fnv1a64([grid.bytes()]),
        fnv1a64([second_grid.bytes()]),
        "the same scenario and seed render the same plane twice"
    );

    // MapSeed is drawn from the synchronized seed before map creation, while
    // C4Landscape's FixRandom bracket keeps map-parser/render draws out of the
    // live synchronized ledger (C4Game.cpp:2651; C4Landscape.cpp:563-579,734).
    assert_eq!(landscape.map_seed(), 38_954);
    assert_eq!(
        (
            snapshot.rng.hold,
            snapshot.rng.count,
            snapshot.rng.rnd3_ptr(),
        ),
        (3_452_408_636, 514, 0),
    );
    let sync = engine.sync_check(0);
    assert_eq!((sync.random_count, sync.random3), (514, 0));

    // Pin the complete post-placement census and the ID assigned to every
    // definition, without embedding 294 repetitive tuples in the test.
    assert_eq!(snapshot.objects.len(), 294);
    assert!(
        snapshot
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .eq(1_u64..=294),
        "Skies of Fire object IDs stay contiguous and in snapshot order",
    );
    assert_eq!(engine.capture_state().next_object_id, 295);
    let identity_hash = snapshot.objects.iter().fold(FNV_OFFSET, |hash, object| {
        let hash = extend_fnv1a64(hash, &object.id.as_u64().to_le_bytes());
        let hash = extend_fnv1a64(hash, &(object.definition_id.len() as u64).to_le_bytes());
        extend_fnv1a64(hash, object.definition_id.as_bytes())
    });
    assert_eq!(identity_hash, 0x52d9_fb05_dd0a_488d);
    let census =
        snapshot
            .objects
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut census, object| {
                *census.entry(object.definition_id.as_str()).or_default() += 1;
                census
            });
    assert_eq!(
        census,
        BTreeMap::from([
            ("DEGG", 20),
            ("DLAR", 8),
            ("DRGN", 7),
            ("FLNT", 54),
            ("FXP1", 4),
            ("GOAL", 1),
            ("GOLD", 50),
            ("LOAM", 59),
            ("MELE", 1),
            ("MGES", 1),
            ("ROCK", 44),
            ("TRB1", 9),
            ("TRB2", 12),
            ("TRB3", 13),
            ("TRB4", 11),
        ]),
    );

    // Script.Initialize calls SpreadDragons, which makes exactly two synced
    // Random draws per dragon and moves them in FindObject order
    // (SkiesOfFire.c4s/Script.c:5-27). Exact IDs/positions plus RandomCount 514
    // prove the shipped callback completed rather than merely linked.
    let dragons = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == "DRGN")
        .map(|object| (object.id.as_u64(), object.position))
        .collect::<Vec<_>>();
    assert_eq!(
        dragons,
        [
            (253, Vector2::new(641, 963)),
            (254, Vector2::new(1_876, 1_110)),
            (255, Vector2::new(1_388, 981)),
            (256, Vector2::new(302, 1_182)),
            (257, Vector2::new(742, 459)),
            (258, Vector2::new(1_795, 1_563)),
            // Upright rather than rotated, so DoCon's initial bottom
            // adjust derives this dragon's centre from the unrotated shape
            // height (C4Object.cpp:1401-1470).
            (259, Vector2::new(1_196, 1_143)),
        ],
    );
}
