const ORDER_FUNC_RESORT_SCRIPT: &str = r#"#strict 2
static pResortLogger, fQueuedReentrantResort, iSynchronousCategorySortCalls;
static pRankProbe, iRankSwapCalls;

local iResortCallCount;
local iResortPair0, iResortPair1, iResortPair2, iResortPair3;
local iResortPair4, iResortPair5, iResortPair6, iResortPair7;
local iResortPair8, iResortPair9, iResortPair10, iResortPair11;
local iSawContained;
local iSectorCount, iSector0, iSector1, iSector2, iSector3, iSector4;

func ArmResortLogger()
{
    pResortLogger = this();
    return ResetResortLog();
}

func ResetResortLog()
{
    iResortCallCount = 0;
    iResortPair0 = iResortPair1 = iResortPair2 = iResortPair3 = 0;
    iResortPair4 = iResortPair5 = iResortPair6 = iResortPair7 = 0;
    iResortPair8 = iResortPair9 = iResortPair10 = iResortPair11 = 0;
    iSawContained = 0;
    iSectorCount = -1;
    iSector0 = iSector1 = iSector2 = iSector3 = iSector4 = 0;
    return true;
}

func RecordResortPair(int iTag, object pLeft, object pRight)
{
    var iPair = iTag + GetX(pLeft) * 10 + GetX(pRight);
    if (iResortCallCount == 0) iResortPair0 = iPair;
    else if (iResortCallCount == 1) iResortPair1 = iPair;
    else if (iResortCallCount == 2) iResortPair2 = iPair;
    else if (iResortCallCount == 3) iResortPair3 = iPair;
    else if (iResortCallCount == 4) iResortPair4 = iPair;
    else if (iResortCallCount == 5) iResortPair5 = iPair;
    else if (iResortCallCount == 6) iResortPair6 = iPair;
    else if (iResortCallCount == 7) iResortPair7 = iPair;
    else if (iResortCallCount == 8) iResortPair8 = iPair;
    else if (iResortCallCount == 9) iResortPair9 = iPair;
    else if (iResortCallCount == 10) iResortPair10 = iPair;
    else if (iResortCallCount == 11) iResortPair11 = iPair;
    iResortCallCount += 1;
    return true;
}

func RecordCrossCheckPair(object pLeft, object pRight)
{
    RecordResortPair(0, pLeft, pRight);
    if (Contained(pLeft) || Contained(pRight)) iSawContained = 1;
    return true;
}

func ReadResortLog()
{
    return [iResortCallCount,
            iResortPair0, iResortPair1, iResortPair2, iResortPair3,
            iResortPair4, iResortPair5, iResortPair6, iResortPair7,
            iResortPair8, iResortPair9, iResortPair10, iResortPair11];
}

func ReadSawContained() { return iSawContained; }

func RecordSectorOrder(aObjects)
{
    if (iSectorCount >= 0) return true;
    iSectorCount = GetLength(aObjects);
    if (iSectorCount > 0) iSector0 = GetX(aObjects[0]);
    if (iSectorCount > 1) iSector1 = GetX(aObjects[1]);
    if (iSectorCount > 2) iSector2 = GetX(aObjects[2]);
    if (iSectorCount > 3) iSector3 = GetX(aObjects[3]);
    if (iSectorCount > 4) iSector4 = GetX(aObjects[4]);
    return true;
}

func ReadSectorOrder()
{
    return [iSectorCount, iSector0, iSector1, iSector2, iSector3, iSector4];
}

func QueueWholeResort()
{
    ResortObjects("ResortOrder", C4D_Object);
    return true;
}

func QueueObjectResort(object pObject)
{
    ResortObject("ResortOrder", pObject);
    return true;
}

func QueueNewestFirstResorts()
{
    ResortObjects("ResortAscending", C4D_Object);
    ResortObjects("ResortDescending", C4D_Object);
    return true;
}

func QueueCrossCheckResort()
{
    ResortObjects("ResortObserveCrossCheck", C4D_Object);
    return true;
}

func QueueSectorVisibilityResorts()
{
    ResortObjects("ResortObserveSector", C4D_Object);
    ResortObjects("ResortOrder", C4D_Object);
    return true;
}

func QueueSectorObserver()
{
    ResortObjects("ResortObserveSector", C4D_Object);
    return true;
}

func QueueReentrantResort()
{
    fQueuedReentrantResort = 0;
    ResortObjects("ResortReentrant", C4D_Object);
    return true;
}

func QueueSynchronousCategorySort()
{
    iSynchronousCategorySortCalls = 0;
    ResortObjects("ResortSynchronousCategorySort", C4D_Object);
    return true;
}

func QueueSwapRankProbe(object pProbe)
{
    pRankProbe = pProbe;
    iRankSwapCalls = 0;
    ResortObjects("ResortSwapRankProbe", C4D_Object);
    return true;
}

func ResortOrder(object pLeft, object pRight)
{
    pResortLogger->RecordResortPair(0, pLeft, pRight);
    return GetY(pLeft) - GetY(pRight);
}

func ResortAscending(object pLeft, object pRight)
{
    pResortLogger->RecordResortPair(100, pLeft, pRight);
    return GetY(pLeft) - GetY(pRight);
}

func ResortDescending(object pLeft, object pRight)
{
    pResortLogger->RecordResortPair(200, pLeft, pRight);
    return GetY(pRight) - GetY(pLeft);
}

func ResortObserveCrossCheck(object pLeft, object pRight)
{
    pResortLogger->RecordCrossCheckPair(pLeft, pRight);
    return 0;
}

func ResortObserveSector(object pLeft, object pRight)
{
    var aObjects = FindObjects([10, 0, 0, 200, 200], [20, "RSRT"]);
    pResortLogger->RecordSectorOrder(aObjects);
    return 0;
}

func ResortReentrant(object pLeft, object pRight)
{
    pResortLogger->RecordResortPair(0, pLeft, pRight);
    if (!fQueuedReentrantResort)
    {
        fQueuedReentrantResort = 1;
        Resort(pLeft);
    }
    return GetY(pLeft) - GetY(pRight);
}

func ResortSynchronousCategorySort(object pLeft, object pRight)
{
    iSynchronousCategorySortCalls += 1;
    if (iSynchronousCategorySortCalls == 1)
    {
        Resort();
        var aObjects = FindObjects([20, "RSRT"]);
        pResortLogger->RecordSectorOrder(aObjects);
    }
    return 0;
}

func ResortSwapRankProbe(object pLeft, object pRight)
{
    iRankSwapCalls += 1;
    if (iRankSwapCalls == 2) SetPosition(3, 3, pRankProbe);
    else if (iRankSwapCalls == 3)
    {
        var aObjects = FindObjects([10, 0, 0, 50, 50], [20, "RSRT"]);
        pResortLogger->RecordSectorOrder(aObjects);
    }
    return GetY(pLeft) - GetY(pRight);
}
"#;

fn spawn_at_fixture(engine: &mut Engine, definition_id: &str, x: i32, y: i32) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_position: Vector2::new(x, y))
}

fn spawn_category_at_fixture(
    engine: &mut Engine,
    definition_id: &str,
    category: i32,
    x: i32,
    y: i32,
) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_category: category, with_position: Vector2::new(x, y))
}

fn spawn_category_in(
    engine: &mut Engine,
    definition_id: &str,
    category: i32,
    container: ObjectId,
) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_category: category, with_container: container)
}

fn spawn_living_at_fixture(engine: &mut Engine, definition_id: &str, x: i32, y: i32) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_alive: true, with_position: Vector2::new(x, y))
}

fn spawn_crew_at_fixture(
    engine: &mut Engine,
    definition_id: &str,
    owner: i32,
    x: i32,
    y: i32,
) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_owner: owner, with_alive: true, with_crew_member: true, with_position: Vector2::new(x, y))
}

fn spawn_full_in(engine: &mut Engine, definition_id: &str, container: ObjectId) -> ObjectId {
    spawn_fixture!(engine, definition_id, with_construction: FULL_CON, with_container: container)
}

fn constructed_living_spawn(
    definition_id: &str,
    x: i32,
    y: i32,
    action: ActionState,
) -> SpawnConfig {
    SpawnConfig::new(definition_id)
        .with_alive(true)
        .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
        .with_construction(FULL_CON)
        .with_position(Vector2::new(x, y))
        .with_action(action)
}

fn c4_definition(id: &str, name: &str, script: &str) -> Definition {
    let mut definition = test_definition(id, name, script);
    definition.set_c4_callback_convention(true);
    definition
}

fn seeded_engine_fixture(seed: u64, definitions: impl IntoIterator<Item = Definition>) -> Engine {
    let mut engine = Engine::with_seed(seed);
    for definition in definitions {
        engine.register_test_definition(definition);
    }
    engine
}

fn engine_fixture(definitions: impl IntoIterator<Item = Definition>) -> Engine {
    let mut engine = Engine::new();
    for definition in definitions {
        engine.register_test_definition(definition);
    }
    engine
}

fn order_engine_fixture(definitions: impl IntoIterator<Item = Definition>) -> Engine {
    let mut engine = seeded_engine_fixture(0, definitions);
    engine.set_landscape(Landscape::flat(100, 100));
    engine
}

fn stateful_engine_fixture(seed: u64) -> Engine {
    seeded_engine_fixture(
        seed,
        [test_definition("Stateful", "Stateful", STATEFUL_SCRIPT)],
    )
}

fn grab_fixture(item_x: i32) -> Result<(Engine, ObjectId, ObjectId), EngineError> {
    let mut crew = test_definition("Crew", "Crew", BASIC_OBJECT_SCRIPT);
    crew.set_crew_member(true);
    crew.set_movement_profile(MovementProfile::default());
    let mut item = test_definition("Gem", "Gem", BASIC_OBJECT_SCRIPT);
    item.set_ocf_base(ocf::GRAB | ocf::CARRYABLE);
    let mut engine = engine_fixture([crew, item]);
    let crew = spawn_crew_at_fixture(&mut engine, "Crew", 1, 0, 16);
    engine.select_crew(1, vec![crew])?;
    engine.set_crew_cursor(1, Some(crew))?;
    let item = spawn_at_fixture(&mut engine, "Gem", item_x, 0);
    Ok((engine, crew, item))
}

fn auto_collect_fixture<const N: usize>(
    collection_limit: Option<i32>,
    item_xs: [i32; N],
) -> (Engine, ObjectId, [ObjectId; N]) {
    let mut crew = test_definition("Crew", "Crew", BASIC_OBJECT_SCRIPT);
    crew.set_crew_member(true);
    crew.set_shape_rect(Some(DefinitionRect::new(-8, -16, 16, 32)));
    crew.set_collection_rect(Some(DefinitionRect::new(-6, -12, 12, 24)));
    if let Some(limit) = collection_limit {
        crew.set_collection_limit(limit);
    }
    let mut item = test_definition("Gem", "Gem", BASIC_OBJECT_SCRIPT);
    item.set_collectible(true);
    let mut engine = engine_fixture([crew, item]);
    let crew = spawn_crew_at_fixture(&mut engine, "Crew", 1, 0, 16);
    let items = item_xs.map(|x| spawn_at_fixture(&mut engine, "Gem", x, 0));
    (engine, crew, items)
}

fn advance_fixture(engine: &mut Engine, frames: usize) -> Result<(), EngineError> {
    for _ in 0..frames {
        engine.tick_without_snapshot()?;
    }
    Ok(())
}

fn order_func_resort_fixture(
    exec_spawn_order: &[(i32, i32)],
) -> (Engine, ObjectId, HashMap<i32, ObjectId>) {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(200, 200));
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_script_definition("RLOG", "Resort logger", ORDER_FUNC_RESORT_SCRIPT);
    engine.register_test_script_definition("RSRT", "Resort target", "#strict 2\n");

    let logger = spawn_category_at_fixture(&mut engine, "RLOG", CATEGORY_STATIC_BACK, 10, 10);
    let mut ids = HashMap::new();
    for &(label, key) in exec_spawn_order {
        let id = spawn_category_at_fixture(&mut engine, "RSRT", CATEGORY_OBJECT, label, key);
        unit_assert!(ids.insert(label, id).is_none(), "labels are unique");
    }
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "ArmResortLogger", Vec::new()) => Value::Bool(true));
    (engine, logger, ids)
}

fn order_func_resort_call(
    engine: &mut Engine,
    logger: ObjectId,
    function: &str,
    args: Vec<Value>,
) -> Value {
    let logger_index = engine.test_object_index(logger);
    engine
        .call_object_function(logger_index, function, args)
        .unwrap_or_else(|error| panic!("{function} succeeds: {error}"))
}

fn order_func_resort_ids(ids: &HashMap<i32, ObjectId>, labels: &[i32]) -> Vec<ObjectId> {
    labels.iter().map(|label| ids[label]).collect()
}

fn order_func_resort_exec_order(engine: &Engine, ids: &HashMap<i32, ObjectId>) -> Vec<ObjectId> {
    engine
        .debug_exec_order()
        .into_iter()
        .filter(|id| ids.values().any(|target| target == id))
        .collect()
}

fn order_func_resort_master_order(engine: &Engine, ids: &HashMap<i32, ObjectId>) -> Vec<ObjectId> {
    engine
        .debug_exec_order()
        .into_iter()
        .rev()
        .filter(|id| ids.values().any(|target| target == id))
        .collect()
}

fn sector_object_ids(engine: &Engine, x: i32, y: i32) -> &[ObjectId] {
    engine
        .sectors
        .as_ref()
        .test_value()
        .object_ids(sector::SectorKey::Inside { x, y })
}

fn order_func_resort_sector_order(engine: &Engine, ids: &HashMap<i32, ObjectId>) -> Vec<ObjectId> {
    sector_object_ids(engine, 0, 0)
        .iter()
        .copied()
        .filter(|id| ids.values().any(|target| target == id))
        .collect()
}

fn order_func_resort_pairs(engine: &mut Engine, logger: ObjectId) -> Vec<i32> {
    let Value::Array(values) = order_func_resort_call(engine, logger, "ReadResortLog", Vec::new())
    else {
        panic!("resort log must be an array")
    };
    let Some(Value::Int(count)) = values.first() else {
        panic!("resort log count must be an integer")
    };
    let count = usize::try_from(*count).test_value();
    unit_assert!(count <= values.len().saturating_sub(1));
    values
        .into_iter()
        .skip(1)
        .take(count)
        .map(|value| match value {
            Value::Int(pair) => pair,
            other => panic!("resort pair must be an integer, got {other:?}"),
        })
        .collect()
}

fn order_func_resort_sector_snapshot(engine: &mut Engine, logger: ObjectId) -> Vec<i32> {
    let Value::Array(values) =
        order_func_resort_call(engine, logger, "ReadSectorOrder", Vec::new())
    else {
        panic!("sector-order log must be an array")
    };
    let Some(Value::Int(count)) = values.first() else {
        panic!("sector-order count must be an integer")
    };
    let count = usize::try_from(*count).test_value();
    values
        .into_iter()
        .skip(1)
        .take(count)
        .map(|value| match value {
            Value::Int(label) => label,
            other => panic!("sector-order label must be an integer, got {other:?}"),
        })
        .collect()
}

#[test]
fn order_func_resort_objects_pins_cpp_comparisons_and_all_three_orders() {
    // C4ObjResort::Sort walks C++ First->Last [4,1,3,2] from the back
    // on every pass. `debug_exec_order` is that master list reversed,
    // hence the deliberately odd spawn/exec order below.
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[2, 3, 1, 4]));
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[4, 1, 3, 2]));

    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueWholeResort", Vec::new()) => Value::Bool(true));
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[2, 3, 1, 4]), "ResortObjects is deferred");
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(order_func_resort_pairs(&mut engine, logger) => vec![23, 21, 14, 32, 24, 34], "each payload swap keeps comparing the same right-hand object");
    let final_master = order_func_resort_ids(&ids, &[1, 2, 3, 4]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => final_master);
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[4, 3, 2, 1]));
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => final_master);
}

#[test]
fn order_func_payload_swap_refreshes_sector_ranks_before_later_update() {
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    let probe_index = engine.test_object_index(ids[&3]);
    engine.objects[probe_index].state.position = Vector2::new(80, 3);
    engine.set_landscape(Landscape::flat(200, 200));
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => order_func_resort_ids(&ids, &[4, 1, 2]), "the probe begins outside the observed sector");
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueSwapRankProbe", vec![Value::Object(ids[&3].as_u64())],) => Value::Bool(true));

    engine.execute_object_order_commands();

    unit_assert_eq!(
        order_func_resort_sector_snapshot(&mut engine, logger) =>
        vec![4, 1, 2, 3],
        "the first payload swap refreshes ranks before the second comparator moves the probe"
    );
}

#[test]
fn order_func_resort_object_forward_walk_continues_ties_and_readds_sector() {
    // Master labels/keys: S=(1,4), A=(2,1), tie=(3,4), B=(4,3),
    // stop=(5,5). Forward calls use (candidate,S); zero continues but
    // is not a move anchor, and the first positive result stops.
    let (mut engine, logger, ids) =
        order_func_resort_fixture(&[(5, 5), (4, 3), (3, 4), (2, 1), (1, 4)]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[1, 2, 3, 4, 5]));

    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueObjectResort", vec![Value::Object(ids[&1].as_u64())],) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(order_func_resort_pairs(&mut engine, logger) => vec![21, 31, 41, 51]);
    let final_master = order_func_resort_ids(&ids, &[2, 3, 4, 1, 5]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => final_master);
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[5, 1, 4, 3, 2]));
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => final_master);
}

#[test]
fn order_func_resort_object_backward_walk_uses_reversed_arguments_and_readds_sector() {
    // Master labels/keys: low=(1,1), A=(2,5), tie=(3,4), S=(4,4),
    // next=(5,6). The forward positive result selects the backward
    // scan, whose calls use (S,candidate); ties continue without moving.
    let (mut engine, logger, ids) =
        order_func_resort_fixture(&[(5, 6), (4, 4), (3, 4), (2, 5), (1, 1)]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[1, 2, 3, 4, 5]));

    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueObjectResort", vec![Value::Object(ids[&4].as_u64())],) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(order_func_resort_pairs(&mut engine, logger) => vec![54, 43, 42, 41]);
    let final_master = order_func_resort_ids(&ids, &[1, 4, 2, 3, 5]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => final_master);
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[5, 3, 2, 4, 1]));
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => final_master);
}

#[test]
fn order_func_resort_requests_execute_newest_first() {
    // Queue ascending first and descending second. Head insertion makes
    // descending execute first; the older ascending request therefore
    // owns the final order. FIFO would leave the exact opposite result.
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(4, 4), (3, 3), (2, 2), (1, 1)]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[1, 2, 3, 4]));
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueNewestFirstResorts", Vec::new(),) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(
        order_func_resort_pairs(&mut engine, logger) =>
        vec![
            243, 242, 241, 232, 231, 221, // newest descending request
            112, 113, 114, 123, 124, 134, // older ascending request
        ]
    );
    let final_master = order_func_resort_ids(&ids, &[1, 2, 3, 4]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => final_master);
    unit_assert_eq!(order_func_resort_exec_order(&engine, &ids) => order_func_resort_ids(&ids, &[4, 3, 2, 1]));
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => final_master);
}

#[test]
fn order_func_next_request_bounded_find_sees_prior_update_pos_resort_order() {
    // The newer ascending request executes first. Its UpdatePosResort
    // must be reflected in the older observer's freshly built host-world
    // sector lists; inspecting Engine.sectors only after both requests
    // would miss the callback-visible stale-order bug. Spawn/storage
    // order is [2,3,1,4], deliberately distinct from the sorted result.
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[4, 1, 3, 2]));
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueSectorVisibilityResorts", Vec::new(),) => Value::Bool(true));

    engine.game_start_synchronize().test_value();

    unit_assert_eq!(
        order_func_resort_sector_snapshot(&mut engine, logger) =>
        vec![1, 2, 3, 4],
        "bounded FindObjects in the next OrderFunc sees the prior request's sector re-adds"
    );
}

#[test]
fn order_func_after_native_category_sort_sees_presort_physical_sector_order() {
    // Native global Resort() sorts Game.Objects by category and refreshes
    // only C4LSectors' master-rank oracle. It does not remove/re-add the
    // existing sector links, so a later OrderFunc callback in the same
    // ExecuteResorts sweep must still enumerate their pre-sort order.
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    for (label, category) in [
        (4, CATEGORY_STATIC_BACK),
        (1, CATEGORY_OBJECT),
        (3, CATEGORY_OBJECT),
        (2, CATEGORY_STRUCTURE),
    ] {
        let index = engine.test_object_index(ids[&label]);
        engine.objects[index].state.category = category;
    }
    let physical_sector_order = order_func_resort_ids(&ids, &[4, 1, 3, 2]);
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => physical_sector_order, "all targets begin in one deliberately non-category-sorted sector list");

    engine
        .load_scenario_script_with_convention(
            "Native category resort probe",
            "#strict 2\nfunc QueueNativeCategorySort() { Resort(); return true; }",
            true,
        )
        .test_value();
    engine
        .call_scenario_script_function("QueueNativeCategorySort", Vec::new())
        .test_value();
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueSectorObserver", Vec::new()) => Value::Bool(true));

    engine.execute_object_order_commands();

    unit_assert_eq!(
        order_func_resort_master_order(&engine, &ids) =>
        order_func_resort_ids(&ids, &[1, 3, 2, 4]),
        "native SortByCategory changes the forward master order"
    );
    unit_assert_eq!(order_func_resort_sector_snapshot(&mut engine, logger) => vec![4, 1, 3, 2], "bounded FindObjects observes the unchanged physical sector links");
    unit_assert_eq!(order_func_resort_sector_order(&engine, &ids) => physical_sector_order, "the zero comparator performs no UpdatePosResort sector rebuild");
}

#[test]
fn order_func_global_resort_sorts_synchronously_before_next_comparison() {
    // The three Object-category links already occupy the correct high
    // bracket, while StaticBack and Structure are reversed below them.
    // Resort() in the first comparator must swap those lower brackets
    // before the same script body performs its boundless master walk.
    let (mut engine, logger, ids) =
        order_func_resort_fixture(&[(5, 5), (4, 4), (2, 2), (3, 3), (1, 1)]);
    for (label, category) in [
        (1, CATEGORY_OBJECT),
        (3, CATEGORY_OBJECT),
        (2, CATEGORY_OBJECT),
        (4, CATEGORY_STATIC_BACK),
        (5, CATEGORY_STRUCTURE),
    ] {
        let index = engine.test_object_index(ids[&label]);
        engine.objects[index].state.category = category;
    }
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => order_func_resort_ids(&ids, &[1, 3, 2, 4, 5]));
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueSynchronousCategorySort", Vec::new(),) => Value::Bool(true));

    engine.execute_object_order_commands();

    let category_sorted = order_func_resort_ids(&ids, &[1, 3, 2, 5, 4]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => category_sorted, "global Resort() sorts immediately at the comparator call boundary");
    unit_assert_eq!(
        order_func_resort_sector_snapshot(&mut engine, logger) =>
        vec![1, 3, 2, 5, 4],
        "later script in the same comparator sees the synchronous category order"
    );
    unit_assert!(engine.pending_object_order_commands.is_empty(), "comparator-global Resort() must not survive as a deferred SortByCategory");
}

#[test]
fn order_func_whole_sort_treats_inactive_exec_hole_as_absent_and_fixed() {
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    let inactive = spawn_fixture!(engine, "RSRT", with_category: CATEGORY_STRUCTURE, with_position: Vector2::new(9, 9), with_status: ObjectStatus::Inactive, with_loaded: true);
    let physical_master = [ids[&4], inactive, ids[&1], ids[&3], ids[&2]];
    engine.exec_list = std::iter::once(logger)
        .chain(physical_master.iter().rev().copied())
        .collect();

    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueWholeResort", Vec::new()) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(
        order_func_resort_pairs(&mut engine, logger) =>
        vec![23, 21, 14, 32, 24, 34],
        "the nonmatching inactive category is transparent to the whole-sort extent"
    );
    unit_assert_eq!(
        engine
            .debug_exec_order()
            .into_iter()
            .rev()
            .filter(|id| *id == inactive || ids.values().any(|target| target == id))
            .collect::<Vec<_>>() =>
        vec![ids[&1], inactive, ids[&2], ids[&3], ids[&4]],
        "active payloads sort across the absent C++ link while its Rust ledger slot stays fixed"
    );
}

#[test]
fn order_func_single_sort_crosses_inactive_exec_hole_without_moving_it() {
    let (mut engine, logger, ids) =
        order_func_resort_fixture(&[(5, 5), (4, 3), (3, 4), (2, 1), (1, 4)]);
    let inactive = spawn_fixture!(engine, "RSRT", with_category: CATEGORY_STRUCTURE, with_position: Vector2::new(9, 9), with_status: ObjectStatus::Inactive, with_loaded: true);
    let physical_master = [ids[&1], inactive, ids[&2], ids[&3], ids[&4], ids[&5]];
    engine.exec_list = std::iter::once(logger)
        .chain(physical_master.iter().rev().copied())
        .collect();

    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueObjectResort", vec![Value::Object(ids[&1].as_u64())],) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    unit_assert_eq!(order_func_resort_pairs(&mut engine, logger) => vec![21, 31, 41, 51]);
    unit_assert_eq!(
        engine
            .debug_exec_order()
            .into_iter()
            .rev()
            .filter(|id| *id == inactive || ids.values().any(|target| target == id))
            .collect::<Vec<_>>() =>
        vec![ids[&2], inactive, ids[&3], ids[&4], ids[&1], ids[&5]],
        "the inactive object is absent from comparisons and retains its unified ledger slot"
    );
}

#[test]
fn reentrant_resort_trigger_does_not_remark_object_after_whole_sort_cleanup() {
    let (mut engine, logger, ids) = order_func_resort_fixture(&[(2, 2), (3, 3), (1, 1), (4, 4)]);
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueReentrantResort", Vec::new()) => Value::Bool(true));
    engine.game_start_synchronize().test_value();

    let sorted_master = order_func_resort_ids(&ids, &[1, 2, 3, 4]);
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => sorted_master);
    unit_assert!(
        !engine.objects[engine.find_object_index(ids[&2]).expect("target remains")].unsorted,
        "whole Sort cleanup consumes the reentrant Resort object's Unsorted flag"
    );
    unit_assert_eq!(
        engine.pending_object_order_commands =>
        [ObjectOrderCommand::ResortUnsortedSweep],
        "only the already-armed global sweep survives ExecuteResorts"
    );

    engine.execute_object_order_commands();
    unit_assert_eq!(order_func_resort_master_order(&engine, &ids) => sorted_master, "the retained trigger must not recreate the consumed Unsorted flag");
    unit_assert!(engine.pending_object_order_commands.is_empty());
}

#[test]
fn order_func_resort_runs_after_same_frame_cross_check() {
    let (mut engine, logger, _ids) = order_func_resort_fixture(&[(2, 2), (1, 1)]);
    let mut collector = simple_definition("RCLL");
    collector.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    collector.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    engine.register_test_definition(collector);
    let mut item_definition = simple_definition("RITM");
    item_definition.set_category(CATEGORY_OBJECT);
    item_definition.set_collectible(true);
    engine.register_test_definition(item_definition);

    let collector = spawn_fixture!(engine, "RCLL", with_category: CATEGORY_LIVING, with_alive: true, with_position: Vector2::new(50, 60));
    let item = spawn_category_at_fixture(&mut engine, "RITM", CATEGORY_OBJECT, 50, 50);

    engine.tick_without_snapshot().test_value();
    engine.tick_without_snapshot().test_value();
    unit_assert_eq!(engine.object_snapshot(item).expect("item remains").container => None, "collection is Tick3-gated");
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "ResetResortLog", Vec::new()) => Value::Bool(true));
    unit_assert_eq!(order_func_resort_call(&mut engine, logger, "QueueCrossCheckResort", Vec::new(),) => Value::Bool(true));

    engine.tick_without_snapshot().test_value();

    unit_assert_eq!(engine.object_snapshot(item).expect("item remains").container => Some(collector));
    unit_assert_eq!(
        order_func_resort_call(&mut engine, logger, "ReadSawContained", Vec::new()) =>
        Value::Int(1),
        "OrderFunc sees the collection CrossCheck performed earlier in frame three"
    );
    unit_assert!(!order_func_resort_pairs(&mut engine, logger).is_empty(), "the containment observation came from a real comparator call");
}

#[test]
fn set_object_order_defers_lifo_and_persists_exec_order_like_cpp() {
    // FnSetObjectOrder pushes C4ObjResort at the ResortProc head
    // (C4Script.cpp:5090-5111); ExecuteResorts consumes head-first after
    // CrossCheck / during Synchronize (C4Game.cpp:1611-1616,
    // C4GameObjects.cpp:874-886). Saves serialize the main list in reverse
    // execution order (C4ObjectList.cpp:506-529).
    let script = r#"
#strict
func Reorder(pRelative, pSort, fAfter) {
    return SetObjectOrder(pRelative, pSort, fAfter);
}
"#;
    let mut engine = order_engine_fixture([
        test_definition("A", "A", script),
        simple_definition("B"),
        simple_definition("C"),
    ]);
    let a = spawn_fixture!(engine, "A", with_category: CATEGORY_OBJECT);
    let b = spawn_fixture!(engine, "B", with_category: CATEGORY_OBJECT);
    let c = spawn_fixture!(engine, "C", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![a, b, c]);

    let a_index = engine.test_object_index(a);
    for args in [
        vec![
            Value::Object(c.as_u64()),
            Value::Object(a.as_u64()),
            Value::Bool(false),
        ],
        vec![
            Value::Object(a.as_u64()),
            Value::Object(c.as_u64()),
            Value::Bool(true),
        ],
    ] {
        unit_assert_eq!(engine.call_object_function(a_index, "Reorder", args).expect("resort queues") => Value::Bool(true));
    }
    unit_assert_eq!(engine.debug_exec_order() => vec![a, b, c], "resort is deferred");

    engine.game_start_synchronize().test_value();
    unit_assert_eq!(engine.debug_exec_order() => vec![c, a, b], "newest request executes first");
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[b, a, c], "sector traversal follows the C++ master-list order");

    // OrderObjectBefore accepts an already-satisfied relationship
    // without moving it (C4ObjectList.cpp:777-780), but its wrapper still
    // calls UpdatePosResort. Stage a current-position change without
    // touching the initially consistent sector map so the re-add is
    // observable in a different sector.
    engine.objects[a_index].state.position = Vector2::new(60, 0);
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[b, a, c]);
    unit_assert_eq!(
        engine
            .call_object_function(
                a_index,
                "Reorder",
                vec![
                    Value::Object(c.as_u64()),
                    Value::Object(a.as_u64()),
                    Value::Bool(false),
                ],
            )
            .expect("satisfied resort queues") =>
        Value::Bool(true)
    );
    engine.game_start_synchronize().test_value();
    unit_assert_eq!(engine.debug_exec_order() => vec![c, a, b]);
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[b, c], "the satisfied relation removes its target's old sector link");
    unit_assert_eq!(sector_object_ids(&engine, 1, 0) => &[a], "the satisfied relation still re-adds its target at the current position");

    let b_index = engine.test_object_index(b);
    // C4ObjResort::Execute also skips a still-normal sort object whose
    // Unsorted bit is pending (for example, after ChangeDef).
    engine.objects[b_index].unsorted = true;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SetRelative {
            relative_to: c,
            object: b,
            after: true,
        });
    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![c, a, b]);
    unit_assert!(engine.objects[b_index].unsorted);
    engine.objects[b_index].unsorted = false;

    // C4ObjResort::Execute skips a sort object that is no longer normal
    // (C4GameObjects.cpp:360-369).
    engine.objects[b_index].state.status = ObjectStatus::Inactive;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SetRelative {
            relative_to: c,
            object: b,
            after: true,
        });
    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![c, a, b]);
    engine.objects[b_index].state.status = ObjectStatus::Normal;

    let state = engine.capture_state();
    unit_assert_eq!(state.object_order => vec![c, a, b]);
    let snapshot = engine.snapshot();
    unit_assert_eq!(
        snapshot
            .objects
            .iter()
            .map(|object| object.id)
            .collect::<Vec<_>>() =>
        vec![a, b, c],
        "deterministic snapshot consumers keep canonical id order"
    );
    unit_assert_eq!(snapshot.render_order => vec![c, a, b], "rendering follows C++ Last -> Prev after SetObjectOrder");
    let mut restored = order_engine_fixture([
        test_definition("A", "A", script),
        simple_definition("B"),
        simple_definition("C"),
    ]);
    restored.restore_state(&state).test_value();
    unit_assert_eq!(restored.debug_exec_order() => vec![c, a, b]);
}

#[test]
fn resort_object_readds_into_its_category_and_definition_cluster() {
    let script = r#"
#strict
func ResortArrow(object target) { target->Resort(); }
func ResortExplicit(object target) { Resort(target); }
"#;
    let mut engine =
        order_engine_fixture([test_definition("A", "A", script), simple_definition("B")]);
    let a1 = spawn_fixture!(engine, "A", with_category: CATEGORY_OBJECT);
    let b = spawn_fixture!(engine, "B", with_category: CATEGORY_OBJECT);
    let a2 = spawn_fixture!(engine, "A", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![a1, a2, b]);

    for function in ["ResortArrow", "ResortExplicit"] {
        engine
            .pending_object_order_commands
            .push(ObjectOrderCommand::SetRelative {
                relative_to: b,
                object: a2,
                after: false,
            });
        engine.execute_object_order_commands();
        unit_assert_eq!(engine.debug_exec_order() => vec![a1, b, a2]);

        let a1_index = engine.test_object_index(a1);
        unit_assert_eq!(engine.call_object_function(a1_index, function, vec![Value::Object(a2.as_u64())],).expect("Resort call succeeds") => Value::Nil);
        unit_assert_eq!(engine.debug_exec_order() => vec![a1, b, a2], "per-object Resort is deferred");
        unit_assert_eq!(engine.pending_object_order_commands => [ObjectOrderCommand::ResortObject(a2)]);
        engine.execute_object_order_commands();
        unit_assert_eq!(engine.debug_exec_order() => vec![a1, a2, b]);
    }

    engine.pending_object_order_commands.extend([
        ObjectOrderCommand::ResortObject(a1),
        ObjectOrderCommand::ResortObject(a2),
    ]);
    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![b, a2, a1], "still-unsorted peers are ignored during each C++ re-add");
}

#[test]
fn set_category_resorts_after_cross_check_before_same_frame_world_phases() {
    // C4Object::SetCategory calls Resort, and C4Game::ExecObjects consumes
    // that global request after CrossCheck but before the remaining world
    // phases. The full stMain re-add puts X at the master-list front of
    // its new category bracket, which is the exec-list end of the bracket.
    let mut mover = test_definition(
        "MOVA",
        "Mover",
        "#strict\nfunc Reclassify() { SetCategory(C4D_Object); return(1); }\n",
    );
    mover.set_timer(1);
    mover.set_timer_call(Some("Reclassify".to_string()));

    let mut engine =
        order_engine_fixture([mover, simple_definition("OBJA"), simple_definition("OBJB")]);

    let mover = spawn_fixture!(engine, "MOVA", with_category: CATEGORY_VEHICLE);
    let first = spawn_fixture!(engine, "OBJA", with_category: CATEGORY_OBJECT);
    let second = spawn_fixture!(engine, "OBJB", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![mover, first, second]);

    engine.tick_without_snapshot().test_value();

    let mover_index = engine.test_object_index(mover);
    unit_assert_eq!(engine.objects[mover_index].state.category & CATEGORY_SORT_LIMIT => CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![first, second, mover], "the post-CrossCheck sweep is visible before the frame's world phases and next exec");
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[mover, second, first], "sector traversal follows the re-added C++ master-list link");
}

#[test]
fn set_category_keeps_a_multi_bit_sort_mask_and_resorts_by_its_raw_value() {
    let mover = test_definition("MOVE", "Mover", "#strict\nfunc Reclassify() { SetCategory(C4D_Structure | C4D_Vehicle); return(GetCategory()); }\nfunc ReadCategory() { return(GetCategory()); }\n");

    let mut engine = order_engine_fixture([
        mover,
        simple_definition("STRU"),
        simple_definition("VEHI"),
        simple_definition("LIVE"),
    ]);

    let structure = spawn_fixture!(engine, "STRU", with_category: CATEGORY_STRUCTURE);
    let vehicle = spawn_fixture!(engine, "VEHI", with_category: CATEGORY_VEHICLE);
    let living = spawn_fixture!(engine, "LIVE", with_category: CATEGORY_LIVING);
    let mover = spawn_fixture!(engine, "MOVE", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![structure, vehicle, living, mover]);

    let raw_category = CATEGORY_STRUCTURE | CATEGORY_VEHICLE;
    let mover_index = engine.test_object_index(mover);
    unit_assert_eq!(engine.call_object_function(mover_index, "Reclassify", Vec::new()).expect("SetCategory succeeds") => Value::Int(raw_category));
    unit_assert_eq!(engine.objects[mover_index].state.category => raw_category, "SetCategory stores the complete requested sort mask");
    unit_assert_eq!(engine.debug_exec_order() => vec![structure, vehicle, living, mover], "SetCategory defers its main-list re-add");

    engine.execute_object_order_commands();
    let raw_order = vec![structure, vehicle, mover, living];
    unit_assert_eq!(engine.debug_exec_order() => raw_order, "C4ObjectList::Add compares the raw 6 mask between Vehicle 4 and Living 8");

    engine.tick_without_snapshot().test_value();
    let mover_index = engine.test_object_index(mover);
    unit_assert_eq!(engine.call_object_function(mover_index, "ReadCategory", Vec::new()).expect("GetCategory succeeds") => Value::Int(raw_category));
    unit_assert_eq!(engine.debug_exec_order() => raw_order);
}

#[test]
fn set_category_resort_prefers_the_same_definition_cluster() {
    // stMain pass 1 takes precedence over the category-bracket fallback:
    // after X moves Vehicle -> Object it is inserted at the same-def
    // cluster head in master order, not at the whole Object bracket head.
    let same = test_definition(
        "SAME",
        "Same",
        "#strict\nfunc Reclassify() { return(SetCategory(C4D_Object)); }\n",
    );
    let mut engine = order_engine_fixture([same, simple_definition("DIFF")]);

    let mover = spawn_fixture!(engine, "SAME", with_category: CATEGORY_VEHICLE);
    let peer = spawn_fixture!(engine, "SAME", with_category: CATEGORY_OBJECT);
    let separator = spawn_fixture!(engine, "DIFF", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![mover, peer, separator]);

    let mover_index = engine.test_object_index(mover);
    unit_assert_eq!(engine.call_object_function(mover_index, "Reclassify", Vec::new()).expect("SetCategory succeeds") => Value::Bool(true));
    unit_assert_eq!(engine.debug_exec_order() => vec![mover, peer, separator], "SetCategory leaves the link in place until the post-CrossCheck seam");

    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![peer, mover, separator]);
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[separator, mover, peer], "the same-def cluster order propagates to sector traversal");
}

#[test]
fn same_frame_spawn_ignores_a_destroyed_definition_cluster_anchor() {
    // AssignRemoval clears Status before returning, but the dead master
    // link remains until the frame-end removal pass. C4ObjectList::Add
    // must skip that link in both scans. Keep another definition between
    // the dead SAME link and its live SAME peer so a stale cluster anchor
    // would put the newcomer on the opposite side of both timer objects.
    let mut same = simple_definition("SAME");
    same.set_category(CATEGORY_OBJECT);
    let mut killer = test_definition(
        "KILL",
        "Killer",
        "#strict\nlocal victim;\nfunc Cull() { RemoveObject(victim); }\n",
    );
    killer.set_category(CATEGORY_OBJECT);
    killer.set_timer(1);
    killer.set_timer_call(Some("Cull".to_string()));
    let mut spawner = test_definition(
        "SPWN",
        "Spawner",
        "#strict\nfunc Seed() { CreateObject(SAME, 0, 0, -1); }\n",
    );
    spawner.set_category(CATEGORY_OBJECT);
    spawner.set_timer(1);
    spawner.set_timer_call(Some("Seed".to_string()));

    let mut engine = order_engine_fixture([same, killer, spawner]);

    let survivor = spawn_fixture!(engine, "SAME", with_category: CATEGORY_OBJECT);
    let victim = spawn_fixture!(engine, "SAME", with_category: CATEGORY_OBJECT);
    let killer = spawn_fixture!(engine, "KILL", with_category: CATEGORY_OBJECT, with_local_vars: HashMap::from([(
        "victim".to_string(),
        Value::Object(victim.as_u64()),
    )]));
    let spawner = spawn_fixture!(engine, "SPWN", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![survivor, victim, killer, spawner]);

    // Main-list Before is exec-list After. Put the future tombstone at
    // the master front so it is the first apparent SAME cluster anchor.
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SetRelative {
            relative_to: spawner,
            object: victim,
            after: false,
        });
    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![survivor, killer, spawner, victim]);

    engine.tick_without_snapshot().test_value();

    unit_assert!(engine.object_snapshot(victim).is_none(), "the killed cluster anchor is no longer a live snapshot object");
    let newcomer = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "SAME" && object.id != survivor)
        .test_value();
    unit_assert_eq!(newcomer.state.timer => 0, "insertion behind the live cursor keeps the newborn out of its birth-frame exec");
    let newcomer = newcomer.id;
    unit_assert_eq!(
        engine.debug_exec_order() =>
        vec![survivor, newcomer, killer, spawner, victim],
        "the dead link remains physical but cannot anchor the SAME definition cluster"
    );
    unit_assert_eq!(sector_object_ids(&engine, 0, 0) => &[spawner, killer, newcomer, survivor], "sector traversal follows the surviving master-list order");
}

#[test]
fn inactive_exec_list_holes_do_not_participate_in_add_or_unsorted_sweeps() {
    // C4OS_INACTIVE objects live in C4GameObjects::InactiveObjects, not
    // the main list. Rust keeps their ids as inert exec-list holes, so a
    // runtime Add must ignore them as cluster candidates and the main
    // ResortUnsorted sweep must neither move nor clear them.
    let mut engine = seeded_engine_fixture(0, ["LOWR", "SAME", "HIGH"].map(simple_definition));

    let low = spawn_fixture!(engine, "LOWR", with_category: CATEGORY_STATIC_BACK);
    let inactive = spawn_fixture!(engine, "SAME", with_category: CATEGORY_OBJECT, with_status: ObjectStatus::Inactive, with_loaded: true);
    let high = spawn_fixture!(engine, "HIGH", with_category: CATEGORY_OBJECT);
    unit_assert_eq!(engine.debug_exec_order() => vec![low, inactive, high]);

    let newcomer = spawn_fixture!(engine, "SAME", with_category: CATEGORY_OBJECT);
    let active_order = engine
        .debug_exec_order()
        .into_iter()
        .filter(|id| {
            engine
                .find_object_index(*id)
                .is_some_and(|index| engine.objects[index].state.status.is_active())
        })
        .collect::<Vec<_>>();
    unit_assert_eq!(active_order => vec![low, high, newcomer], "an inactive same-def hole is not a main-list cluster candidate");

    let inactive_position = engine
        .debug_exec_order()
        .iter()
        .position(|id| *id == inactive)
        .test_value();
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SetRelative {
            relative_to: newcomer,
            object: high,
            after: false,
        });
    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![low, inactive, newcomer, high], "SetObjectOrder moves only logical main-list links across an inactive hole");
    unit_assert_eq!(
        engine
            .debug_exec_order()
            .iter()
            .position(|id| *id == inactive) =>
        Some(inactive_position),
        "SetObjectOrder preserves the unified inactive ledger slot"
    );

    let inactive_position = engine
        .debug_exec_order()
        .iter()
        .position(|id| *id == inactive)
        .test_value();
    let inactive_index = engine.test_object_index(inactive);
    engine.objects[inactive_index].unsorted = true;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::ResortObject(high));
    engine.execute_object_order_commands();

    let inactive_index = engine.test_object_index(inactive);
    unit_assert_eq!(
        engine
            .debug_exec_order()
            .iter()
            .position(|id| *id == inactive) =>
        Some(inactive_position),
        "the main-list sweep leaves the inactive hole in place"
    );
    unit_assert!(engine.objects[inactive_index].unsorted, "the main-list sweep does not consume an inactive object's flag");

    // The no-object Resort path calls SortByCategory on Game.Objects
    // only. Give the inactive hole a deliberately low raw category: it
    // still must not be included in that stable main-list sort.
    engine.objects[inactive_index].state.category = 0;
    let inactive_position = engine
        .debug_exec_order()
        .iter()
        .position(|id| *id == inactive)
        .test_value();
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();
    unit_assert_eq!(
        engine
            .debug_exec_order()
            .iter()
            .position(|id| *id == inactive) =>
        Some(inactive_position),
        "global SortByCategory also leaves inactive holes fixed"
    );
}

#[test]
fn contents_add_inserts_after_the_last_sorted_predecessor_before_an_unsorted_peer() {
    // C4ObjectList::Add skips Unsorted links while comparing categories,
    // but inserts at cPrev->Next. An Unsorted link between the last sorted
    // predecessor and the qualifying successor therefore remains AFTER
    // the newcomer; inserting directly at the successor would be too late.
    let mut engine = seeded_engine_fixture(
        0,
        ["CONT", "HIGH", "UNST", "LOWR", "NEWW"].map(simple_definition),
    );
    let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
    let high = spawn_category_in(&mut engine, "HIGH", CATEGORY_OBJECT, container);
    let unsorted = spawn_category_in(&mut engine, "UNST", CATEGORY_LIVING, container);
    let low = spawn_category_in(&mut engine, "LOWR", CATEGORY_STRUCTURE, container);
    let container_index = engine.test_object_index(container);
    unit_assert_eq!(engine.objects[container_index].state.contents => vec![high, unsorted, low]);

    let unsorted_index = engine.test_object_index(unsorted);
    engine.objects[unsorted_index].unsorted = true;
    let newcomer = spawn_category_in(&mut engine, "NEWW", CATEGORY_VEHICLE, container);

    let container_index = engine.test_object_index(container);
    unit_assert_eq!(engine.objects[container_index].state.contents => vec![high, newcomer, unsorted, low]);
}

#[test]
fn global_resort_stably_sorts_by_raw_masked_category() {
    let mut engine = order_engine_fixture(["A", "B", "C", "D", "E"].map(simple_definition));
    let ids = ["A", "B", "C", "D", "E"]
        .map(|definition| spawn_fixture!(engine, definition, with_category: CATEGORY_OBJECT));
    let [a, b, c, d, e] = ids;
    for (id, category) in [
        (a, CATEGORY_VEHICLE),
        (b, CATEGORY_STATIC_BACK),
        (c, CATEGORY_VEHICLE),
        (d, CATEGORY_STRUCTURE),
        (e, CATEGORY_LIVING | CATEGORY_OBJECT),
    ] {
        let index = engine.test_object_index(id);
        engine.objects[index].state.category = category;
    }
    unit_assert_eq!(engine.debug_exec_order() => vec![a, b, c, d, e]);

    engine
        .load_scenario_script_with_convention(
            "Resort probe",
            "#strict\nfunc Sort() { Resort(); }",
            true,
        )
        .test_value();
    engine
        .call_scenario_script_function("Sort", Vec::new())
        .test_value();
    unit_assert_eq!(engine.debug_exec_order() => vec![a, b, c, d, e]);
    unit_assert_eq!(engine.pending_object_order_commands => [ObjectOrderCommand::SortByCategory]);

    engine.execute_object_order_commands();
    unit_assert_eq!(engine.debug_exec_order() => vec![b, d, a, c, e]);
    unit_assert_eq!(engine.snapshot().render_order => vec![b, d, a, c, e]);
    let e_index = engine.test_object_index(e);
    unit_assert_eq!(
        engine.objects[e_index].state.category & CATEGORY_SORT_LIMIT =>
        CATEGORY_LIVING | CATEGORY_OBJECT,
        "SortByCategory compares the raw mask without normalizing it"
    );
}

// The GoldRush intro wall (f22 talker x 30-vs-28): C4ObjectList::Add
// stMain pass 1 (C4ObjectList.cpp:150-162) inserts a new object BEFORE
// the first same-sorted-category, same-def link — and ExecObjects walks
// the list BACKWARDS (C4Game.cpp:1582), so a runtime spawn of an
// existing def executes right AFTER the last-executing member of its
// def cluster, NOT at the end of its category bracket. The intro _TLK
// talker therefore execs BEFORE the later-joined TRPR player and its
// DFA_ATTACH reads the player's PREVIOUS-frame position (one-frame
// lag); rust's global-creation order read the same-frame position.
#[test]
fn runtime_spawn_clusters_with_existing_def_in_exec_order_like_cpp() {
    let mut mover = test_definition("Wagn", "Wagon", "#strict\n");
    mover.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    let mut crew = test_definition("Crew", "Crew", "#strict\n");
    crew.configure_actions(
        None,
        HashMap::from([(
            "Ride".to_string(),
            ActionSpec::default().with_procedure("ATTACH"),
        )]),
    );
    let mut talk = test_definition("Talk", "Talker", "#strict\n");
    talk.configure_actions(
        None,
        HashMap::from([(
            "Attach".to_string(),
            ActionSpec::default().with_procedure("ATTACH"),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 100, -100));
    engine.register_test_definition(mover);
    engine.register_test_definition(crew);
    engine.register_test_definition(talk);

    // Creation order mirrors GoldRush: pre-placed talker cluster,
    // then the player, then the intro talker.
    let _talker_zero = spawn_category_at_fixture(&mut engine, "Talk", CATEGORY_LIVING, 300, 50);
    let rider = spawn_category_at_fixture(&mut engine, "Crew", CATEGORY_LIVING, 100, 50);
    let vehicle = spawn_category_at_fixture(&mut engine, "Wagn", CATEGORY_VEHICLE, 100, 50);
    let intro_talker = spawn_category_at_fixture(&mut engine, "Talk", CATEGORY_LIVING, 100, 50);
    engine
        .apply_object_update(
            rider,
            ObjectUpdate::new().with_action_update(
                ActionUpdate::default()
                    .with_name("Ride")
                    .with_force(true)
                    .with_target(Some(vehicle)),
            ),
        )
        .test_value();
    engine
        .apply_object_update(
            intro_talker,
            ObjectUpdate::new().with_action_update(
                ActionUpdate::default()
                    .with_name("Attach")
                    .with_force(true)
                    .with_target(Some(rider)),
            ),
        )
        .test_value();

    // The horse's pull, reduced to a plain 2px/frame roll.
    let vehicle_idx = engine.test_object_index(vehicle);
    engine.objects[vehicle_idx].fixed_velocity.x = itofix(2);
    engine.objects[vehicle_idx].state.mobile = true;

    let x_of = |engine: &Engine, id| {
        engine
            .find_object_index(id)
            .map(|idx| engine.objects[idx].state.position.x)
            .test_value()
    };

    engine.tick_without_snapshot().test_value();
    unit_assert_eq!(x_of(&engine, vehicle) => 102, "vehicle rolls 2px");
    unit_assert_eq!(x_of(&engine, rider) => 102, "rider execs after the vehicle: attach reads the post-move x");
    unit_assert_eq!(
        x_of(&engine, intro_talker) =>
        100,
        "clustered talker execs BEFORE the rider (C4ObjectList.cpp:150-162 \
             + reverse exec, C4Game.cpp:1582): reads the PRE-exec rider x"
    );

    engine.tick_without_snapshot().test_value();
    unit_assert_eq!(x_of(&engine, vehicle) => 104);
    unit_assert_eq!(x_of(&engine, rider) => 104);
    unit_assert_eq!(x_of(&engine, intro_talker) => 102, "one-frame lag persists while the vehicle moves");
}

// Builds the GoldRush coach as a movement fixture: shape/vertices/
// frictions from Coach.c4d DefCore (wheels y=+15 friction 10,
// body vertices friction 100/80/80), UprightAttach=8, Mass=150.
fn coach_fixture_definition() -> Definition {
    let mut coach = test_definition("Wagn", "Wagon", "#strict\n");
    coach.set_shape_rect(Some(DefinitionRect::new(-27, -20, 55, 40)));
    coach.set_shape_vertices(vec![
        ObjectVertex::new(0, 1).with_friction(100),
        ObjectVertex::new(-15, -6).with_cnat(5).with_friction(80),
        ObjectVertex::new(15, -6).with_cnat(6).with_friction(80),
        ObjectVertex::new(-16, 15).with_cnat(9).with_friction(10),
        ObjectVertex::new(16, 15).with_cnat(10).with_friction(10),
    ]);
    coach.set_upright_attach(CNAT_BOTTOM as i32);
    coach.set_mass(150);
    coach.set_grab(1);
    coach
}

// Spawns the coach fixture resting on ground at (100, 260) — wheels
// (y+15) one pixel above the solid line — and settles it through the
// spawn-frame transients so fix_x/fix_y are pixel-snapped, dirs zero
// and Mobile off (the state C4Object::Push finds a parked wagon in).
fn settled_coach_engine(landscape: Landscape) -> (Engine, ObjectId) {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(landscape);
    // Gravity 100 => GravAccel = FIXED100(100)/5 = raw 13107
    // (C4Landscape.cpp:66).
    engine.set_physics(PhysicsSettings::new(100, 100, -100));
    engine.register_test_definition(coach_fixture_definition());
    let id = spawn_category_at_fixture(&mut engine, "Wagn", CATEGORY_VEHICLE, 100, 260);
    advance_fixture(&mut engine, 3).test_value();
    let idx = engine.test_object_index(id);
    let object = &engine.objects[idx];
    unit_assert_eq!(object.state.position => Vector2::new(100, 260), "settled");
    unit_assert_eq!(object.fixed_velocity.x => C4Fixed::ZERO);
    unit_assert_eq!(object.fixed_velocity.y => C4Fixed::ZERO);
    unit_assert!(!object.state.mobile, "parked wagon is demobilized");
    (engine, id)
}

// The horse's Pull3 push against wheel friction: a pushed 150-mass
// wagon with VertexFriction=10 wheels loses xdir in exact
// ApplyFriction quanta — ffric = FFriction * 10 / 100 = FIXED100(30)
// * 10 / 100 = raw 1966 — once per vertical ground contact
// (C4Movement.cpp:297-317 vertical loop, :50-56 ApplyFriction, :89-96
// ContactVtxFriction takes the FIRST contacted vertex). With gravity
// 0.2/frame the wheels touch down every second frame (fix_y crosses
// the .5 rounding boundary on accumulated 0.6), and the touch-down
// zeroes ydir because both Left|Bottom and Right|Bottom wheels contact
// (C4Movement.cpp:304-317). Horizontal rolling on flat ground is
// contact-free and keeps the remaining xdir.
#[test]
fn pushed_wagon_loses_xdir_by_wheel_friction_quanta_on_ground_contacts() {
    let (mut engine, id) = settled_coach_engine(Landscape::flat(400, 276));
    let idx = engine.test_object_index(id);
    // The horse's first Pull3 push on a parked wagon:
    // dforce = ValByPhysical(250, 100000) * 100 / 150 = raw 109226,
    // Towards(0, txdir=119276) adds the full dforce (C4Object.cpp:1770,
    // 1775-1783). Direct dir writes must arm Mobile like Push does.
    engine.objects[idx].fixed_velocity.x = C4Fixed::from_raw(109226);
    engine.objects[idx].state.mobile = true;

    // Frame A: gravity 0.2 only — no vertical step (fixtoi rounds
    // 260.2 to 260), free horizontal roll, xdir untouched.
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.fixed_velocity.x.val() => 109226, "no contact yet");
    unit_assert_eq!(object.fixed_velocity.y.val() => 13107, "one gravity quantum");
    unit_assert_eq!(object.state.position => Vector2::new(102, 260));

    // Frame B: accumulated fix_y = 260.6 rounds to 261 — the wheels
    // hit the ground: ONE wheel-friction quantum off xdir, ydir zeroed
    // (both wheels contact -> C4Movement.cpp:308-317 else-branch).
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.fixed_velocity.x.val() => 109226 - 1966, "ApplyFriction(xdir, 10) = FIXED100(30)*10/100 = 1966");
    unit_assert_eq!(object.fixed_velocity.y => C4Fixed::ZERO, "touch-down zeroes ydir");
    unit_assert_eq!(object.state.position => Vector2::new(103, 260));

    // Frame C: airborne accumulation again — xdir keeps its value.
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.fixed_velocity.x.val() => 107260);
    unit_assert_eq!(object.fixed_velocity.y.val() => 13107);

    // Frame D: second touch-down, second quantum.
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.fixed_velocity.x.val() => 107260 - 1966);
    unit_assert_eq!(object.fixed_velocity.y => C4Fixed::ZERO);
}

// Horizontal contact: the rolling wagon's leading wheel meets a step —
// the horizontal move aborts (ctcox = x, fix_x snapped), RedirectForce
// moves FIXED100(50) = raw 32768 from xdir into UPWARD ydir, then
// ApplyFriction(ydir, friction of the first contacted vertex — the
// wheel, 10) bleeds raw 1966 back off (C4Movement.cpp:266-282).
#[test]
fn wagon_hitting_a_step_redirects_half_pixel_of_xdir_upward_and_aborts() {
    // Flat road at 276 with a raised solid ledge (surface 240) from
    // x=120: stepping to x=104 puts the right wheel (x+16, y+15) into
    // the ledge column while the body vertices stay clear.
    let library = MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
    )
    .test_value();
    let materials = MaterialSet::from_resource_library(&library);
    let earth = materials.id_of("Earth").test_value();
    let mut surface = vec![276; 400];
    for column in surface.iter_mut().skip(120) {
        *column = 240;
    }
    let landscape = Landscape::with_default_material(400, surface, Some(earth)).test_value();
    let (mut engine, id) = {
        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(100, 100, -100));
        engine.register_test_definition(coach_fixture_definition());
        let id = spawn_category_at_fixture(&mut engine, "Wagn", CATEGORY_VEHICLE, 100, 260);
        advance_fixture(&mut engine, 3).test_value();
        (engine, id)
    };
    let idx = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[idx].state.position => Vector2::new(100, 260), "settled");
    engine.objects[idx].fixed_velocity.x = C4Fixed::from_raw(109226);
    engine.objects[idx].state.mobile = true;

    // fix_x = 101.667 -> ctcox 102: steps to 101 free, 102 free.
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.state.position => Vector2::new(102, 260));
    unit_assert_eq!(object.fixed_velocity.x.val() => 109226);

    // fix_x = 103.33 -> ctcox 103: the step to 103 is free; the
    // touch-down (fix_y = 260.6) costs one wheel quantum.
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.state.position => Vector2::new(103, 260));
    unit_assert_eq!(object.fixed_velocity.x.val() => 109226 - 1966, "wheel touch-down");

    // ctcox 105; the step to 104 contacts (right wheel at 120,275 in
    // the ledge): abort + fix_x snap + RedirectForce + ydir friction
    // of the first contacted vertex (the wheel, friction 10).
    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.state.position.x => 103, "horizontal move aborted");
    unit_assert_eq!(object.fixed_velocity.x.val() => 109226 - 1966 - 32768, "RedirectForce takes min(|xdir|, FIXED100(50)) out of xdir");
    unit_assert_eq!(object.fixed_velocity.y.val() => 13107 - 32768 + 1966, "redirected upward, then ApplyFriction(ydir, wheel friction 10)");
    unit_assert_eq!(object.fixed_position.x.val() => itofix(103).val(), "horizontal abort snaps fix_x to the integer pixel");
}

// The phase-end NextAction transition runs through SetAction INSIDE
// ExecAction (C4Object.cpp:5473-5480), which resyncs fix_x/fix_y to
// the integer position (:4154-4155) and runs the new action's
// StartCall INLINE (:4160-4171) — all BEFORE ExecMovement
// (C4Object::Execute order, C4Object.cpp:1074/1079). A StartCall that
// SetActions again (the GoldRush coach's Driving -> "Drive2") snaps
// the fixed coords at the PRE-movement pixel, so the frame's xdir
// still lands on top of the snap: fix_x ends at itofix(x0) + xdir.
// Deferring the StartCall until after DoMovement would snap away the
// sub-pixel remainder the movement just accumulated (the GoldRush
// coach wall: cpp 67.209 vs rust 67.000 at the Turn->Drive2 wrap).
#[test]
fn phase_wrap_start_call_set_action_resyncs_fixed_coords_before_movement() {
    let script = r#"#strict
protected func StartGlide() { SetAction("Glide2"); return(1); }
"#;
    let mut coach = coach_fixture_definition();
    let mut wrap_def = c4_definition("Wagn", "Wagon", script);
    wrap_def.set_shape_rect(coach.shape_rect());
    wrap_def.set_shape_vertices(coach.shape_vertices().to_vec());
    wrap_def.set_upright_attach(CNAT_BOTTOM as i32);
    wrap_def.set_mass(150);
    wrap_def.set_grab(1);
    let mut actions = HashMap::new();
    // Roll wraps on its first exec (Delay 1, Length 1) into Glide,
    // whose StartCall immediately SetActions to Glide2 — the coach's
    // Turn -> Drive0 -> Driving() -> "Drive2" chain in miniature.
    actions.insert(
        "Roll".to_string(),
        ActionSpec::default()
            .with_delay(1)
            .with_length(1)
            .with_next("Glide"),
    );
    actions.insert(
        "Glide".to_string(),
        ActionSpec::default().with_start_call("StartGlide"),
    );
    actions.insert("Glide2".to_string(), ActionSpec::default());
    wrap_def.configure_actions(None, actions);
    coach = wrap_def;

    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 276));
    engine.set_physics(PhysicsSettings::new(100, 100, -100));
    engine.register_test_definition(coach);
    let id = spawn_category_at_fixture(&mut engine, "Wagn", CATEGORY_VEHICLE, 100, 260);
    advance_fixture(&mut engine, 3).test_value();
    let idx = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[idx].state.position => Vector2::new(100, 260));
    unit_assert_eq!(engine.objects[idx].fixed_position.x => itofix(100), "settled");

    // Arm the wrap frame: rolling right at 0.7 px/frame in Roll's
    // final phase tick.
    engine.objects[idx].state.action = ActionState::new("Roll");
    engine.objects[idx].fixed_velocity.x = C4Fixed::from_raw(45875);
    engine.objects[idx].state.mobile = true;

    engine.tick_without_snapshot().test_value();
    let object = &engine.objects[engine.test_object_index(id)];
    unit_assert_eq!(object.state.action.name => "Glide2", "wrap ran Glide's StartCall, which SetActioned Glide2");
    unit_assert_eq!(object.state.position.x => 101, "fixtoi(100.7) rounds to 101");
    unit_assert_eq!(
        object.fixed_position.x.val() =>
        itofix(100).val() + 45875,
        "SetAction's fix_x resync (C4Object.cpp:4155) happens BEFORE \
             DoMovement adds xdir — the sub-pixel remainder survives the wrap"
    );
}

// The attached do-loop applies the at_xovr/at_yovr override
// bookkeeping AFTER the contact arm, UNCONDITIONALLY
// (C4Movement.cpp:355-368): when Shape.Attach adjusts the step
// (at_yovr=1) and the contact check at the adjusted candidate then
// HITS, C++ aborts (fix snap, :359-361) and STILL zeroes ydir via
// the override (:367-368) — the object demobilizes. Rust broke out
// of the loop on contact before the override ran, keeping the
// re-arm frame's gravity quantum alive: the f138 wall — the resting
// coach cycled (ydir 13107, Mobile true) in rust where cpp rested
// (0, false), and every contained cargo copied the difference.
#[test]
fn attached_contact_after_attach_adjustment_still_zeroes_ydir_like_cpp() {
    let mut engine = Engine::with_seed(0);
    let library = MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
    )
    .test_value();
    let materials = MaterialSet::from_resource_library(&library);
    let earth = materials.id_of("Earth").test_value();
    engine.set_materials(materials);
    // Ground under the wheel column at 117 (wheel vertex rests 2px
    // above -> Attach pulls DOWN one, ycnt=+1); a pillar under the
    // seat column at 102 blocks the pulled-down candidate.
    let mut surface = vec![117i32; 200];
    surface[124] = 102;
    let landscape = Landscape::with_default_material(200, surface, Some(earth)).test_value();
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(100, 100, -100));

    let mut wagon = test_definition("Wagn", "Wagon", "#strict\n");
    wagon.set_shape_rect(Some(DefinitionRect::new(-27, -20, 55, 40)));
    wagon.set_shape_vertices(vec![
        ObjectVertex::new(24, 1).with_friction(100),
        ObjectVertex::new(0, 15)
            .with_cnat(CNAT_BOTTOM)
            .with_friction(10),
    ]);
    wagon.set_upright_attach(CNAT_BOTTOM as i32);
    let mut actions = HashMap::new();
    actions.insert("Stand".to_string(), ActionSpec::default());
    wagon.configure_actions(Some("Stand".to_string()), actions);
    engine.register_test_definition(wagon);

    let id = spawn_fixture!(engine, "Wagn", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(100, 100), with_loaded: true);
    let idx = engine.test_object_index(id);
    // A parked wagon: dirs zero, fixed coords pixel-exact, Mobile off
    // — the UprightAttach re-arm (C4Object.cpp:4712-4718) mobilizes
    // it next ExecAction and the default-arm DoGravity adds one
    // quantum.
    engine.objects[idx].fixed_velocity = FixedVec2::ZERO;
    engine.objects[idx].state.velocity = Vector2::ZERO;
    engine.objects[idx].state.mobile = false;

    engine.tick_without_snapshot().test_value();
    let idx = engine.test_object_index(id);
    let object = &engine.objects[idx];
    unit_assert_eq!(object.state.position => Vector2::new(100, 100), "no motion");
    unit_assert_eq!(
        object.fixed_velocity.y =>
        C4Fixed::ZERO,
        "the at_yovr override zeroes ydir even though the adjusted step \
             contacted (C4Movement.cpp:367-368 runs after the abort arm)"
    );
    unit_assert_eq!(object.fixed_position.y => itofix(100), "abort resynced fix_y");
    unit_assert!(!object.state.mobile, "all dirs zero after the override -> demobilized (C4Movement.cpp:577)");
}

// Pins the Pull3 arithmetic the GoldRush horse feeds C4Object::Push
// (C4Object.cpp:5110-5131): Pulling3 sets the temp Walk physical to
// 130000 (Horse.c4d Script.c), fWalk = ValByPhysical(280, 130000) =
// itofix(130000*56, 2000000) = raw 238551; with the wagon 5px past the
// pull position (BoundBy(iPullX - target.x) = -5, COMD_Right) the
// target force is fWalk + fWalk*(-5)/10 = 238551 - 119275 = raw 119276
// — the C4Fixed integer division truncates toward zero on the negative
// product. The push force on the 150-mass wagon is
// ValByPhysical(250, 100000)*100/150 = raw 109226.
#[test]
fn pull3_walk_physical_yields_the_goldrush_pull_forces() {
    let walk = math::val_by_physical(280, 130_000);
    unit_assert_eq!(walk.val() => 238551);
    let txdir = walk + walk * (-5) / 10;
    unit_assert_eq!(txdir.val() => 119276, "negative product truncates toward zero");
    let dforce = math::val_by_physical(250, 100_000) * 100 / 150;
    unit_assert_eq!(dforce.val() => 109226);
    // First push on a parked wagon: Towards(0, 119276, 109226) adds the
    // full dforce (C4Object.cpp:1775-1783).
    let mut xdir = C4Fixed::ZERO;
    math::towards(&mut xdir, txdir, dforce);
    unit_assert_eq!(xdir.val() => 109226);
}

// Mirrors the Def TimerCall (C4Object::Execute, C4Object.cpp:1085-1091):
// every object counts Timer++ per frame; reaching Def->Timer resets
// the counter and Execs Def->TimerCall (no pars, fail-safe). DefCore
// Timer= defaults to 35 (C4Def.cpp:298); Objects.txt saves the
// mid-cycle per-object counter (Timer=, default 0, C4Object.cpp:2738).
// C4Object::ExecLife growth (C4Object.cpp:824-837): every Tick35,
// an incomplete StaticBack (or alive Living) with Def Growth and no
// fire gains DoCon(Growth*100); DoCon keeps the shape bottom
// anchored (strgt_con_b, C4Object.cpp:1419,1476-1483) so the
// GoldRush bushes creep upward as they grow (the f35 live class:
// BUSH 1276 y 310 -> 309 in C++ while rust never grew it).
#[test]
fn static_back_growth_ticks_con_and_keeps_the_bottom_anchored() {
    let mut definition = simple_definition("Bush");
    // The real BUSH shape (Bush1.c4d DefCore: 41x39 at -20,-19) —
    // this step (25610 -> 26010) grows the stretched height 9 -> 10
    // while the stretched top offset stays -4, so y shifts -1.
    definition.set_shape_rect(Some(DefinitionRect::new(-20, -19, 41, 39)));
    definition.set_category(CATEGORY_STATIC_BACK);
    definition.set_growth(4);
    definition.set_stretch_growth(true);
    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 200, -200));
    engine.set_environment(EnvironmentSettings::new(0));
    engine.register_test_definition(definition);
    let bush = spawn_fixture!(engine, "Bush", with_position: Vector2::new(100, 310), with_construction: 25610);

    advance_fixture(&mut engine, 34).test_value();
    let idx = engine.test_object_index(bush);
    unit_assert_eq!(engine.objects[idx].state.construction => 25610, "no growth before the Tick35 boundary");
    let y_before = engine.objects[idx].state.position.y;
    engine.tick_without_snapshot().test_value();
    let idx = engine.test_object_index(bush);
    unit_assert_eq!(engine.objects[idx].state.construction => 26010, "DoCon(Growth*100) at Tick35 (C4Object.cpp:837)");
    unit_assert_eq!(engine.objects[idx].state.position.y => y_before - 1, "bottom-anchored stretch shifts y up (C4Object.cpp:1476-1483)");
}

#[test]
fn static_back_growth_full_crossing_runs_callbacks_and_suppresses_after_removal(
) -> Result<(), EngineError> {
    // ExecLife's Tick35 growth delegates to DoCon. Its FullCon crossing
    // runs Completion then Initialize exactly once; a Completion that
    // removes its object suppresses Initialize (C4Object.cpp:824-837,
    // 1506-1511).
    let mut growing = test_definition(
        "GROW",
        "Growing",
        r#"#strict 3
        local lifecycle;
        func Completion() {
            if (!lifecycle) lifecycle = 0;
            lifecycle = lifecycle * 10 + 1;
            return true;
        }
        func Initialize() { lifecycle = lifecycle * 10 + 2; return true; }
        "#,
    );
    growing.set_category(CATEGORY_STATIC_BACK);
    growing.set_growth(1);
    let mut removed = test_definition(
        "GONE",
        "Removed growth",
        r#"#strict 3
        local armed;
        func Arm() { armed = true; return true; }
        func Completion() { RemoveObject(); return true; }
        func Initialize() { if (armed) CreateObject(MARK); return true; }
        "#,
    );
    removed.set_category(CATEGORY_STATIC_BACK);
    removed.set_growth(1);

    let mut engine = Engine::with_seed(131);
    engine.register_definition(growing)?;
    engine.register_definition(removed)?;
    engine.register_definition(simple_definition("MARK"))?;
    let growing_id = engine.spawn_object(
        SpawnConfig::new("GROW")
            .with_category(CATEGORY_STATIC_BACK)
            .with_construction(FULL_CON - 100),
    )?;
    let removed_id = engine.spawn_object(
        SpawnConfig::new("GONE")
            .with_category(CATEGORY_STATIC_BACK)
            .with_construction(FULL_CON - 100),
    )?;
    let removed_index = engine.test_object_index(removed_id);
    engine.call_object_function(removed_index, "Arm", Vec::new())?;
    unit_assert_eq!(engine.snapshot().objects.iter().filter(|object| object.definition_id == "MARK").count() => 0, "partial creation does not run Initialize");

    advance_fixture(&mut engine, 35)?;

    let growing = engine.test_object_snapshot(growing_id);
    unit_assert_eq!(growing.construction => FULL_CON);
    unit_assert_eq!(growing.local_vars.get("lifecycle") => Some(&Value::Int(12)));
    unit_assert!(engine.object_snapshot(removed_id).is_none());
    unit_assert_eq!(engine.snapshot().objects.iter().filter(|object| object.definition_id == "MARK").count() => 0, "removed Completion suppresses Initialize");
    Ok(())
}

#[test]
fn def_timer_call_fires_on_the_def_interval_like_cpp() {
    let script = r#"
        local iTicks;
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        func Tick() { iTicks = iTicks + 1; return 1; }
        "#;
    let mut definition = test_definition("Fire", "Fire", script);
    definition.set_timer(5);
    definition.set_timer_call(Some("Tick".to_string()));
    let mut engine = seeded_engine_fixture(0, [definition]);
    let id = spawn_fixture!(engine, "Fire", with_category: CATEGORY_OBJECT);

    let ticks_of = |engine: &Engine| {
        engine
            .find_object_index(id)
            .and_then(|idx| engine.objects[idx].state.local_vars.get("iTicks").cloned())
    };
    for _ in 1..=4 {
        engine.tick_without_snapshot().test_value();
    }
    unit_assert!(!matches!(ticks_of(&engine), Some(Value::Int(_))), "Timer counts 1..4, below the interval (C4Object.cpp:1086)");
    engine.tick_without_snapshot().test_value();
    unit_assert_eq!(ticks_of(&engine) => Some(Value::Int(1)), "the 5th Execute reaches Def->Timer and fires TimerCall (C4Object.cpp:1086-1090)");
    for _ in 6..=10 {
        engine.tick_without_snapshot().test_value();
    }
    unit_assert_eq!(ticks_of(&engine) => Some(Value::Int(2)), "the counter resets and fires again every interval");
}

// ObjectActionCornerScale changes Swim -> Walk through SetAction before
// returning from movement (C4ObjectCom.cpp:191-217), so Walk's StartCall
// and Swim's AbortCall have both run synchronously (C4Object.cpp:4171-4197)
// before the later Def TimerCall arm in C4Object::Execute
// (C4Object.cpp:1047-1117). A TimerCall SetAction("Sit") must therefore
// dispatch Sitting exactly once, without replaying the movement transition
// against the now-current Sit action.
#[test]
fn movement_action_callbacks_run_before_timer_set_action_like_cpp() {
    let script = r#"#strict
local iWalkStarted;
local iSwimAborted;
local iSitting;
local fMovementCallbacksRanBeforeTimer;
protected func Activity()
{
  fMovementCallbacksRanBeforeTimer = (iWalkStarted == 1 && iSwimAborted == 1);
  SetAction("Sit");
  return(1);
}
private func WalkStart()
{
  iWalkStarted = iWalkStarted + 1;
  return(1);
}
private func SwimAbort()
{
  iSwimAborted = iSwimAborted + 1;
  return(1);
}
private func Sitting()
{
  if (GetEffect("PossessionSpell", this())) return();
  iSitting = iSitting + 1;
  Random(10);
  return(1);
}
"#;
    let mut definition = c4_definition("WIPF", "Wipf", script);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
    definition.set_contact_density(50);
    definition.set_physical(PhysicalInfo {
        swim: 100_000,
        ..PhysicalInfo::default()
    });
    definition.configure_actions(
        Some("Swim".to_string()),
        HashMap::from([
            (
                "Swim".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_abort_call("SwimAbort"),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_start_call("WalkStart"),
            ),
            (
                "Sit".to_string(),
                ActionSpec::default().with_start_call("Sitting"),
            ),
        ]),
    );
    definition.set_timer(1);
    definition.set_timer_call(Some("Activity".to_string()));

    let mut engine = Engine::with_seed(424_242);
    let mut landscape = vehicle_grid_landscape(24, 24);
    landscape.set_world_height(24);
    for x in 0..24 {
        landscape.grid_write_byte(x, 12, 1);
    }
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_definition(definition);
    let wipf = engine.spawn_test_object(
        SpawnConfig::new("WIPF")
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::new(10, 10))
            .with_fixed_position(FixedVec2::from_ints(10, 10))
            .with_action(ActionState::new("Swim"))
            .with_direction(Direction::Right)
            .with_command_direction(CommandDirection::Down)
            .with_mobile(true)
            .with_loaded(true),
    );
    let wipf_idx = engine.test_object_index(wipf);
    engine.objects[wipf_idx].state.in_liquid = true;
    engine.objects[wipf_idx].fixed_velocity.y = itofix(1);
    let count_before = engine.rng.count;

    engine.tick_without_snapshot().test_value();

    let wipf_idx = engine.test_object_index(wipf);
    let locals = &engine.objects[wipf_idx].state.local_vars;
    let observed = (
        locals.get("fMovementCallbacksRanBeforeTimer").cloned(),
        locals.get("iWalkStarted").cloned(),
        locals.get("iSwimAborted").cloned(),
        locals.get("iSitting").cloned(),
        engine.rng.count - count_before,
    );
    unit_assert_eq!(
        observed =>
        (
            Some(Value::Bool(true)),
            Some(Value::Int(1)),
            Some(Value::Int(1)),
            Some(Value::Int(1)),
            1,
        ),
        "CornerScale's Walk StartCall and Swim AbortCall precede TimerCall; \
             the TimerCall's Sitting callback then consumes one Random(10) draw"
    );
}

#[test]
fn timer_spawn_on_remaining_list_side_executes_in_the_same_frame_like_cpp() {
    // C4Game::ExecObjects walks a LIVE reverse-list iterator
    // (src/C4Game.cpp:1588-1597). C4Game::NewObject adds a child by its
    // initial definition category (src/C4Game.cpp:1117-1127), and
    // C4ObjectList::Add places that link on the iterator's remaining side
    // (src/C4ObjectList.cpp:134-175). The pinned GoldRush frame-367
    // differential freezes this with newborn WMPF #1595: C++ Timer=1.
    let mut parent = test_definition("PRNT", "Parent", "#strict\nfunc Seed() { var child = CreateObject(CHLD, 0, 0, -1); child->Place(); return(1); }\n");
    parent.set_category(CATEGORY_STATIC_BACK);
    parent.set_timer(1);
    parent.set_timer_call(Some("Seed".to_string()));

    let mut child = test_definition(
        "CHLD",
        "Child",
        "#strict\nfunc Place() { SetCategory(1); return(1); }\n",
    );
    child.set_category(CATEGORY_OBJECT);
    child.configure_actions(
        Some("Exist".to_string()),
        HashMap::from([("Exist".to_string(), ActionSpec::default())]),
    );

    let mut engine = seeded_engine_fixture(0, [parent, child]);
    engine.spawn_test_object(SpawnConfig::new("PRNT"));

    engine.tick_without_snapshot().test_value();
    let child = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "CHLD")
        .test_value();
    unit_assert_eq!(child.state.timer => 1, "the live reverse iterator executes the newborn child before frame end");
    unit_assert_eq!(child.state.action.time => 1, "ExecAction also ran once");
    unit_assert_eq!(child.state.category => CATEGORY_STATIC_BACK);
}

#[test]
fn spawn_assigns_container_relationships() {
    let mut engine = seeded_engine_fixture(0, ["Crate", "Gem"].map(simple_definition));

    let crate_id = engine.spawn_test_object(SpawnConfig::new("Crate"));
    let gem_id = spawn_fixture!(engine, "Gem", with_container: crate_id);

    let crate_snapshot = engine.test_object_snapshot(crate_id);
    unit_assert_eq!(crate_snapshot.contents => vec![gem_id]);

    let gem_snapshot = engine.test_object_snapshot(gem_id);
    unit_assert_eq!(gem_snapshot.container => Some(crate_id));
}

#[test]
fn object_update_moves_between_containers() {
    let mut engine = seeded_engine_fixture(0, ["Crate", "Chest", "Gem"].map(simple_definition));

    let crate_id = engine.spawn_test_object(SpawnConfig::new("Crate"));
    let chest_id = engine.spawn_test_object(SpawnConfig::new("Chest"));
    let gem_id = spawn_fixture!(engine, "Gem", with_container: crate_id);

    engine
        .apply_object_update(gem_id, ObjectUpdate::new().with_container(chest_id))
        .test_value();

    let crate_snapshot = engine.test_object_snapshot(crate_id);
    unit_assert!(crate_snapshot.contents.is_empty());

    let chest_snapshot = engine.test_object_snapshot(chest_id);
    unit_assert_eq!(chest_snapshot.contents => vec![gem_id]);

    let gem_snapshot = engine.test_object_snapshot(gem_id);
    unit_assert_eq!(gem_snapshot.container => Some(chest_id));
}

#[test]
fn destroying_container_detaches_contents() {
    let mut engine = seeded_engine_fixture(0, ["Crate", "Gem"].map(simple_definition));

    let crate_id = engine.spawn_test_object(SpawnConfig::new("Crate"));
    let gem_id = spawn_fixture!(engine, "Gem", with_container: crate_id);

    engine
        .apply_object_update(
            crate_id,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )
        .test_value();

    let gem_snapshot = engine.test_object_snapshot(gem_id);
    unit_assert_eq!(gem_snapshot.container => None);

    let crate_snapshot = engine.test_object_snapshot(crate_id);
    unit_assert!(crate_snapshot.contents.is_empty());
    unit_assert_eq!(crate_snapshot.status => ObjectStatus::Deleted);
}

#[test]
fn capture_state_restores_container_relationships() {
    let mut engine = seeded_engine_fixture(0, ["Crate", "Gem"].map(simple_definition));

    let crate_id = engine.spawn_test_object(SpawnConfig::new("Crate"));
    let gem_id = spawn_fixture!(engine, "Gem", with_container: crate_id);

    let state = engine.capture_state();

    let mut restored = seeded_engine_fixture(1, ["Crate", "Gem"].map(simple_definition));
    restored.restore_state(&state).test_value();

    let crate_snapshot = restored.test_object_snapshot(crate_id);
    unit_assert_eq!(crate_snapshot.contents => vec![gem_id]);

    let gem_snapshot = restored.test_object_snapshot(gem_id);
    unit_assert_eq!(gem_snapshot.container => Some(crate_id));
}

#[test]
fn step_script_can_enqueue_commands() {
    let script = r#"#strict 3
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        { velocity = [5, 0] },
                        { delay = 1, action = { name = "Slide", phase = 0 } }
                    ]
                };
            }
            return nil;
        }
        "#;

    let mut definition = test_definition("Actor", "Actor", script);
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert("Slide".to_string(), ActionSpec::default());
    definition.configure_actions(Some("Idle".to_string()), actions);

    let mut engine = seeded_engine_fixture(3, [definition]);

    let id = spawn_fixture!(engine, "Actor", with_position: Vector2::new(0, 0), with_velocity: Vector2::new(0, 0));

    let first = engine.test_tick();
    let object = first.object(id).test_value();
    unit_assert_eq!(object.velocity.x => 0);

    let second = engine.test_tick();
    let object = second.object(id).test_value();
    unit_assert_eq!(object.velocity.x => 5);

    let third = engine.test_tick();
    let object = third.object(id).test_value();
    unit_assert_eq!(object.action.name => "Slide");
}

#[test]
fn saves_and_loads_engine_state_via_files() -> Result<(), EngineError> {
    let mut engine = stateful_engine_fixture(0xC0FFEE);
    engine.set_physics(PhysicsSettings::new(3, 8, -5));
    engine.set_environment(EnvironmentSettings::new(5));
    engine.set_landscape(Landscape::flat(48, 12));

    let object_id = engine.spawn_object(
        SpawnConfig::new("Stateful")
            .with_position(Vector2::new(4, 2))
            .with_velocity(Vector2::new(1, -1))
            .with_energy(7),
    )?;

    engine.queue_object_command(
        object_id,
        QueuedCommand::new(1, ObjectUpdate::new().with_velocity(Vector2::new(2, -3))),
    )?;
    engine.tick_without_snapshot()?;

    let state = engine.capture_state();
    let temp_file = NamedTempFile::new().test_value();
    state.save_to_path(temp_file.path()).test_value();

    let loaded = EngineState::load_from_path(temp_file.path()).test_value();
    unit_assert_eq!(loaded.frame => state.frame);
    unit_assert_eq!(loaded.physics => state.physics);
    unit_assert_eq!(loaded.environment => state.environment);
    unit_assert_eq!(loaded.objects.len() => state.objects.len());
    unit_assert_eq!(loaded.global_effects => state.global_effects);
    unit_assert_eq!(loaded.crew_selection => state.crew_selection);
    unit_assert_eq!(loaded.crew_roles => state.crew_roles);
    unit_assert_eq!(loaded.known_crew_owners => state.known_crew_owners);
    unit_assert_eq!(loaded.eliminated_crew_owners => state.eliminated_crew_owners);

    let mut restored = stateful_engine_fixture(77);
    restored.restore_state(&loaded)?;

    let expected = engine.tick()?;
    let mut actual = restored.tick()?;
    unit_assert_eq!(actual.audio => vec![AudioCommand::SetMusicPlaylist {playlist: None, restart: false,}, AudioCommand::SetMusicLevel { level: 100 },]);
    actual.audio.clear();
    unit_assert_eq!(expected => actual);

    Ok(())
}

#[test]
fn engine_state_round_trips_the_sky_scroll_state() -> Result<(), EngineError> {
    // C4Sky::CompileFunc persists x/y/xdir/ydir (C4Sky.cpp:248-251)
    // and the savegame Init keeps the loaded values (`if (!fSavegame)`
    // reset gate, C4Sky.cpp:77-80) — save/load must resume the exact
    // fixed scroll state.
    let mut engine = Engine::with_seed(3);
    engine.set_environment(EnvironmentSettings::new(41));
    let mut settings = SkySettings::default().with_surface(128, 128);
    settings.parallax_mode = SkyParallaxMode::Wind;
    engine.set_sky(settings);
    advance_fixture(&mut engine, 3)?;
    let expected = engine.snapshot().sky;
    let moved = expected
        .as_ref()
        .and_then(|frame| frame.fixed)
        .is_some_and(|fixed| fixed[0] != 0);
    unit_assert!(moved, "wind-mode sky must have scrolled before the save");

    let state = engine.capture_state();
    let mut restored = Engine::with_seed(9);
    restored.restore_state(&state)?;
    unit_assert_eq!(restored.snapshot().sky => expected);
    Ok(())
}

#[test]
fn get_sky_adjust_reads_live_raw_values_and_survives_save_restore() -> Result<(), EngineError> {
    // FnGetSkyAdjust returns Modulation or BackClr according to the bool
    // parameter (C4Script.cpp:4632-4636). SetModulation writes both raw
    // values immediately, even when its alpha byte disables BackClr, and
    // C4Sky::CompileFunc persists all three fields independently
    // (C4Sky.cpp:238-258).
    let script = r#"#strict
local iInitialMod, iInitialBack, iAlphaMod, iAlphaBack;
local iDisabledMod, iDisabledBack, iStringBack, iArrayBack, iNilMod, iExtraBack;
local iRestoredMod, iRestoredBack;

func ProbeSky() {
    var no_value;
    iInitialMod = GetSkyAdjust();
    iInitialBack = GetSkyAdjust(1);
    SetSkyAdjust(-2130706433, 1193046);
    iAlphaMod = GetSkyAdjust();
    iAlphaBack = GetSkyAdjust(1);
    SetSkyAdjust(11259375, 6636321);
    iDisabledMod = GetSkyAdjust(false);
    iDisabledBack = GetSkyAdjust(true);
    iStringBack = GetSkyAdjust("");
    iArrayBack = GetSkyAdjust([]);
    iNilMod = GetSkyAdjust(no_value);
    iExtraBack = GetSkyAdjust(1, 0);
    return(1);
}

func ReadRestoredSky() {
    iRestoredMod = GetSkyAdjust();
    iRestoredBack = GetSkyAdjust(1);
    return(1);
}
"#;
    let mut engine = Engine::with_seed(0);
    engine.register_definition(test_definition("SKYP", "Sky probe", script))?;
    let object_id = engine.spawn_object(SpawnConfig::new("SKYP"))?;
    let object_index = engine.test_object_index(object_id);
    engine.call_object_function(object_index, "ProbeSky", Vec::new())?;

    let locals = &engine.test_object_snapshot(object_id).local_vars;
    unit_assert_eq!(locals.get("iInitialMod") => Some(&Value::Int(0x00ff_ffff)));
    unit_assert_eq!(locals.get("iInitialBack") => Some(&Value::Int(0)));
    unit_assert_eq!(locals.get("iAlphaMod") => Some(&Value::Int(-2_130_706_433)));
    unit_assert_eq!(locals.get("iAlphaBack") => Some(&Value::Int(0x12_3456)));
    unit_assert_eq!(locals.get("iDisabledMod") => Some(&Value::Int(0x00ab_cdef)));
    unit_assert_eq!(locals.get("iDisabledBack") => Some(&Value::Int(0x65_4321)));
    unit_assert_eq!(locals.get("iStringBack") => Some(&Value::Int(0x65_4321)));
    unit_assert_eq!(locals.get("iArrayBack") => Some(&Value::Int(0x65_4321)));
    unit_assert_eq!(locals.get("iNilMod") => Some(&Value::Int(0x00ab_cdef)));
    unit_assert_eq!(locals.get("iExtraBack") => Some(&Value::Int(0x65_4321)));

    let state_json = serde_json::to_string(&engine.capture_state()).test_value();
    let state: EngineState = serde_json::from_str(&state_json).test_value();
    let sky = state.sky.as_ref().test_value();
    unit_assert_eq!(sky.settings.modulation => Some(0x00ab_cdef));
    unit_assert_eq!(sky.settings.back_color => None, "alpha byte disables fill");
    unit_assert_eq!(sky.settings.back_color_raw => 0x65_4321);

    let mut restored = Engine::with_seed(1);
    restored.register_definition(test_definition("SKYP", "Sky probe", script))?;
    restored.restore_state(&state)?;
    let object_index = restored.test_object_index(object_id);
    restored.call_object_function(object_index, "ReadRestoredSky", Vec::new())?;
    let locals = &restored.test_object_snapshot(object_id).local_vars;
    unit_assert_eq!(locals.get("iRestoredMod") => Some(&Value::Int(0x00ab_cdef)));
    unit_assert_eq!(locals.get("iRestoredBack") => Some(&Value::Int(0x65_4321)));
    Ok(())
}

#[test]
fn get_sky_color_matches_legacy_blt_alpha_palette_lookup() -> Result<(), EngineError> {
    // FnGetSkyColor (C4Script.cpp:3056-3069) accepts only index zero.
    // Its alpha is zero, but BltAlpha uses inverted alpha and shifts by
    // eight, so FadeClr2 channels are multiplied by 255/256.
    let script = r#"#strict
local red, green, blue;
local positive_index, negative_index, low_channel, high_channel;

func ProbeSkyColor() {
    red = GetSkyColor(0, 0);
    green = GetSkyColor(0, 1);
    blue = GetSkyColor(0, 2);
    positive_index = GetSkyColor(1, 0);
    negative_index = GetSkyColor(-1, 1);
    low_channel = GetSkyColor(0, -1);
    high_channel = GetSkyColor(0, 3);
    return(1);
}
"#;
    let mut engine = Engine::with_seed(0);
    let mut settings = SkySettings::default();
    settings.fade_top = RgbColor::new(17, 34, 51);
    settings.fade_bottom = RgbColor::new(1, 128, 255);
    engine.set_sky(settings);
    engine.register_definition(test_definition("SKYC", "Sky color probe", script))?;
    let object_id = engine.spawn_object(SpawnConfig::new("SKYC"))?;
    let object_index = engine.test_object_index(object_id);
    engine.call_object_function(object_index, "ProbeSkyColor", Vec::new())?;

    let locals = &engine.test_object_snapshot(object_id).local_vars;
    unit_assert_eq!(locals.get("red") => Some(&Value::Int(0)));
    unit_assert_eq!(locals.get("green") => Some(&Value::Int(127)));
    unit_assert_eq!(locals.get("blue") => Some(&Value::Int(254)));
    for name in [
        "positive_index",
        "negative_index",
        "low_channel",
        "high_channel",
    ] {
        unit_assert_eq!(locals.get(name) => Some(&Value::Int(0)), "{name}");
    }
    Ok(())
}

#[test]
fn set_sky_color_matches_cpp_and_ignores_other_indices() -> Result<(), EngineError> {
    // FnSetSkyColor (C4Script.cpp:3046-3054) is an index-zero-only
    // compatibility shim. Its writes are immediately visible to script
    // and then persist through C4Sky::SetModulation.
    let script = r#"#strict
local noop_result, noop_mod, noop_back;
local changed_result, changed_mod, changed_back;

func ProbeSetSkyColor() {
    noop_result = SetSkyColor(1, 200, 210, 220);
    noop_mod = GetSkyAdjust();
    noop_back = GetSkyAdjust(1);
    changed_result = SetSkyColor(0, 96, 64, 200);
    changed_mod = GetSkyAdjust();
    changed_back = GetSkyAdjust(1);
    return(1);
}
"#;
    let mut engine = Engine::with_seed(0);
    let mut settings = SkySettings::default();
    settings.fade_top = RgbColor::new(64, 128, 200);
    settings.modulation = Some(0x0011_2233);
    settings.back_color = None;
    settings.back_color_raw = 0x0044_5566;
    engine.set_sky(settings);
    engine.register_definition(test_definition("SKYS", "Sky setter probe", script))?;
    let object_id = engine.spawn_object(SpawnConfig::new("SKYS"))?;
    let object_index = engine.test_object_index(object_id);
    engine.call_object_function(object_index, "ProbeSetSkyColor", Vec::new())?;

    let locals = &engine.test_object_snapshot(object_id).local_vars;
    unit_assert_eq!(locals.get("noop_result") => Some(&Value::Nil));
    unit_assert_eq!(locals.get("noop_mod") => Some(&Value::Int(0x0011_2233)));
    unit_assert_eq!(locals.get("noop_back") => Some(&Value::Int(0x0044_5566)));
    unit_assert_eq!(locals.get("changed_result") => Some(&Value::Nil));
    unit_assert_eq!(locals.get("changed_mod") => Some(&Value::Int(0x20ff_80ff)));
    unit_assert_eq!(locals.get("changed_back") => Some(&Value::Int(0x003f_42c8)));

    let sky = engine.capture_state().sky.test_value();
    unit_assert_eq!(sky.settings.fade_top => RgbColor::new(64, 128, 200));
    unit_assert_eq!(sky.settings.modulation => Some(0x20ff_80ff));
    unit_assert_eq!(sky.settings.back_color => Some(0x003f_42c8));
    unit_assert_eq!(sky.settings.back_color_raw => 0x003f_42c8);
    Ok(())
}

#[test]
fn set_sky_fade_uses_only_the_from_color_like_newgfx() -> Result<(), EngineError> {
    // FnSetSkyFade's NewGfx compatibility path ignores its second RGB
    // triple after normal typed-parameter conversion.
    let script = r#"#strict
local result, modulation, back_color;

func ProbeSetSkyFade() {
    result = SetSkyFade(96, 64, 200, 1, 2, 3);
    modulation = GetSkyAdjust();
    back_color = GetSkyAdjust(1);
    return(1);
}
"#;
    let mut engine = Engine::with_seed(0);
    let mut settings = SkySettings::default();
    settings.fade_top = RgbColor::new(64, 128, 200);
    settings.fade_bottom = RgbColor::new(10, 20, 30);
    engine.set_sky(settings);
    engine.register_definition(test_definition("SKYF", "Sky fade probe", script))?;
    let object_id = engine.spawn_object(SpawnConfig::new("SKYF"))?;
    let object_index = engine.test_object_index(object_id);
    engine.call_object_function(object_index, "ProbeSetSkyFade", Vec::new())?;

    let locals = &engine.test_object_snapshot(object_id).local_vars;
    unit_assert_eq!(locals.get("result") => Some(&Value::Nil));
    unit_assert_eq!(locals.get("modulation") => Some(&Value::Int(0x20ff_80ff)));
    unit_assert_eq!(locals.get("back_color") => Some(&Value::Int(0x003f_42c8)));

    let sky = engine.capture_state().sky.test_value();
    unit_assert_eq!(sky.settings.fade_top => RgbColor::new(64, 128, 200));
    unit_assert_eq!(sky.settings.fade_bottom => RgbColor::new(10, 20, 30));
    unit_assert_eq!(sky.settings.modulation => Some(0x20ff_80ff));
    unit_assert_eq!(sky.settings.back_color_raw => 0x003f_42c8);
    Ok(())
}

#[test]
fn captures_and_restores_engine_state() -> Result<(), EngineError> {
    let mut engine = stateful_engine_fixture(0xBAD_F00D);
    engine.set_physics(PhysicsSettings::new(2, 9, -6));
    engine.set_landscape(Landscape::flat(128, 15));
    engine.set_environment(EnvironmentSettings::new(-4));

    let object_id = engine.spawn_object(
        SpawnConfig::new("Stateful")
            .with_position(Vector2::new(10, 5))
            .with_velocity(Vector2::new(1, -2))
            .with_energy(12),
    )?;

    engine.queue_object_command(
        object_id,
        QueuedCommand::new(
            2,
            ObjectUpdate::new()
                .with_action_update(ActionUpdate::default().with_name("Rest").with_phase(4)),
        )
        .with_effects(vec![EffectCommand::add(
            EffectState::new("Glow")
                .with_priority(90)
                .with_interval(3)
                .with_timer(1),
        )])
        .with_spawns(vec![SpawnConfig::new("Stateful")
            .with_position(Vector2::new(3, 0))
            .with_velocity(Vector2::new(0, 0))
            .with_energy(5)
            .with_action(ActionState::new("Helper"))]),
    )?;

    engine.tick_without_snapshot()?;

    let state = engine.capture_state();
    unit_assert_eq!(state.environment => engine.environment());
    let serialized = state.to_json_string().test_value();
    let decoded = EngineState::from_json_str(&serialized).test_value();

    let mut restored = stateful_engine_fixture(123);
    restored.set_physics(PhysicsSettings::new(5, 11, -8));
    restored.set_landscape(Landscape::flat(64, 9));
    restored.set_environment(EnvironmentSettings::new(9));
    restored.restore_state(&decoded)?;

    unit_assert_eq!(restored.physics() => state.physics);
    unit_assert_eq!(restored.environment() => state.environment);
    unit_assert_eq!(restored.landscape() => state.landscape.as_ref());
    unit_assert_eq!(engine.snapshot() => restored.snapshot());

    let next_original = engine.tick()?;
    let mut next_restored = restored.tick()?;
    unit_assert_eq!(
        next_restored.audio =>
        vec![
            AudioCommand::SetMusicPlaylist {
                playlist: None,
                restart: false,
            },
            AudioCommand::SetMusicLevel { level: 100 },
        ],
        "state restore reapplies the default music playlist and level to the frontend"
    );
    next_restored.audio.clear();
    unit_assert_eq!(next_original => next_restored);

    let spawn_original = engine.tick()?;
    let spawn_restored = restored.tick()?;
    unit_assert_eq!(spawn_original => spawn_restored);

    Ok(())
}

#[test]
fn tick_applies_temperature_conversions_to_landscape() -> Result<(), EngineError> {
    let mut engine = Engine::with_seed(0xC0);
    let library = MaterialLibrary::parse(
        r#"
            [Material Ice]
            Name=Ice
            Density=80
            Friction=15
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=4

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
    )
    .test_value();
    engine.configure_materials_from_library(&library);
    let ice = engine.materials().id_of("Ice").test_value();
    engine.set_landscape(Landscape::flat_with_material(4, 10, Some(ice)));
    let environment = EnvironmentSettings::new(0).with_temperature(10);
    engine.set_environment(environment);

    engine.tick_without_snapshot()?;

    let water = engine.materials().id_of("Water").test_value();
    let landscape = engine.landscape().test_value();
    unit_assert_eq!(landscape.solid_material_at(0) => Some(water));
    unit_assert_eq!(landscape.default_solid_material() => Some(water));

    Ok(())
}

#[test]
fn try_grab_nearby_moves_object_into_inventory() -> Result<(), EngineError> {
    // Bottom-anchor the crew at center y=0 (C4Object.cpp:1462-1468).
    let (mut engine, crew, item) = grab_fixture(8)?;

    unit_assert!(engine.try_grab_nearby(1)?);
    let snapshot = engine.test_object_snapshot(item);
    unit_assert_eq!(snapshot.container => Some(crew));
    Ok(())
}

#[test]
fn try_drop_held_object_uses_object_com_drop_for_stop_direction() -> Result<(), EngineError> {
    // Bottom-anchor the crew at center y=0 (C4Object.cpp:1462-1468).
    let (mut engine, crew, item) = grab_fixture(6)?;

    unit_assert!(engine.try_grab_nearby(1)?);
    let crew_before_drop = engine.test_object_snapshot(crew);
    unit_assert!(engine.try_drop_held_object(1)?);
    let item_snapshot = engine.test_object_snapshot(item);
    unit_assert!(item_snapshot.container.is_none(), "item should be released from inventory");
    unit_assert_eq!(item_snapshot.position => crew_before_drop.position, "COMD_Stop has tdir=0 and equal shape bottoms keep the same anchor");
    let item_index = engine.test_object_index(item);
    unit_assert_eq!(engine.objects[item_index].fixed_velocity => FixedVec2::ZERO);
    // Every ObjectComDrop-shaped drop arms the dropper's NoCollectDelay
    // (C4ObjectCom.cpp:668-669) — the helper must not leave the crew
    // instantly recollecting what it just dropped.
    let crew_index = engine.test_object_index(crew);
    unit_assert_eq!(engine.objects[crew_index].state.no_collect_delay => 2);
    Ok(())
}

#[test]
fn object_com_drop_matches_cpp_exit_physics_callback_order_and_ungrab() -> Result<(), EngineError> {
    // ObjectComDrop computes its fixed throw velocity and shape-relative
    // exit point before C4Object::Exit, then Exit dispatches Ejection and
    // Departure before NoCollectDelay/SetOCF and ObjectComUnGrab
    // (C4ObjectCom.cpp:640-676; C4Object.cpp:1513-1563).
    let actor_script = r#"#strict 2
local callback_order, ejected, departed, ungrab_target, target_ungrabbed;
local ejection_had_collection, ejection_item_container;
local ejection_x, ejection_y, ejection_xdir, ejection_ydir, ejection_r, ejection_rdir;
local ejection_no_collect, departure_no_collect, departure_had_collection;
local grab_had_collection, grabbed_had_collection;

protected func Ejection(pObject)
{
  callback_order = callback_order * 10 + 1;
  ejected = pObject;
  ejection_had_collection = !!(GetOCF() & OCF_Collection);
  ejection_item_container = pObject->Contained();
  ejection_x = GetX(pObject);
  ejection_y = GetY(pObject);
  ejection_xdir = GetXDir(pObject, 100);
  ejection_ydir = GetYDir(pObject, 100);
  ejection_r = GetR(pObject);
  ejection_rdir = GetRDir(pObject, 100);
  ejection_no_collect = GetObjectVal("NoCollectDelay");
  return(1);
}

public func NoteDeparture(pObject)
{
  callback_order = callback_order * 10 + 2;
  departed = pObject;
  departure_no_collect = GetObjectVal("NoCollectDelay");
  departure_had_collection = !!(GetOCF() & OCF_Collection);
  return(1);
}

protected func Grab(pTarget, fGrab)
{
  if (!fGrab)
  {
    callback_order = callback_order * 10 + 3;
    ungrab_target = pTarget;
    grab_had_collection = !!(GetOCF() & OCF_Collection);
  }
  return(1);
}

public func NoteGrabbed(pTarget)
{
  callback_order = callback_order * 10 + 4;
  target_ungrabbed = pTarget;
  grabbed_had_collection = !!(GetOCF() & OCF_Collection);
  return(1);
}
"#;
    let item_script = r#"#strict 2
protected func Departure(pContainer)
{
  pContainer->NoteDeparture(this());
  return(1);
}
"#;
    let target_script = r#"#strict 2
protected func Grabbed(pClonk, fGrab)
{
  if (!fGrab) pClonk->NoteGrabbed(this());
  return(1);
}
"#;

    let mut actor = c4_definition("DPAC", "Drop actor", actor_script);
    actor.set_crew_member(true);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_collection_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_physical(PhysicalInfo {
        throw: 50_001,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
        ]),
    );
    let mut item = c4_definition("DPIT", "Drop item", item_script);
    item.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
    let target = c4_definition("DPTG", "Push target", target_script);

    let mut engine = seeded_engine_fixture(175, [actor, item, target]);
    engine.register_player(PlayerConfig::new(1, "Dropper"))?;
    let target_id = engine.spawn_object(
        SpawnConfig::new("DPTG")
            .with_category(CATEGORY_VEHICLE)
            .with_construction(FULL_CON)
            .with_position(Vector2::new(120, 200)),
    )?;
    let mut push_action = ActionState::new("Push");
    push_action.target = Some(target_id);
    let actor_id = engine.spawn_object(
        constructed_living_spawn("DPAC", 100, 200, push_action)
            .with_owner(1)
            .with_crew_member(true)
            .with_velocity(Vector2::new(-2, 0))
            .with_command_direction(CommandDirection::Right),
    )?;
    let item_id = spawn_full_in(&mut engine, "DPIT", actor_id);
    engine.select_crew(1, [actor_id])?;
    engine.set_crew_cursor(1, Some(actor_id))?;

    let actor_index = engine.test_object_index(actor_id);
    engine.refresh_object_ocf(actor_index);
    unit_assert_ne!(engine.objects[actor_index].state.ocf & ocf::COLLECTION => 0, "the Ejection callback must begin with collection still enabled");
    let actor_position = engine.objects[actor_index].state.position;
    let actor_shape = engine.objects[actor_index]
        .current_shape_rect()
        .test_value();
    let item_index = engine.test_object_index(item_id);
    let item_shape = engine.objects[item_index].current_shape_rect().test_value();
    engine.objects[item_index].fixed_velocity = FixedVec2::new(math::itofix(9), -math::itofix(7));
    engine.objects[item_index].state.velocity = Vector2::new(9, -7);
    engine.objects[item_index].state.rotation = 37;
    engine.objects[item_index].fixed_rotation = math::itofix(37);
    engine.objects[item_index].rotation_velocity = math::itofix(5);
    engine.objects[item_index].state.in_liquid = true;
    engine.objects[item_index].state.mobile = false;
    let expected_position = Vector2::new(
        actor_position.x + actor_shape.x + actor_shape.width,
        actor_position.y + actor_shape.y + actor_shape.height - (item_shape.y + item_shape.height),
    );
    let expected_force = math::val_by_physical(400, 50_001);

    unit_assert!(engine.try_drop_held_object(1)?);

    let item_index = engine.test_object_index(item_id);
    unit_assert_eq!(engine.objects[item_index].state.container => None);
    unit_assert_eq!(engine.objects[item_index].state.position => expected_position);
    unit_assert_eq!(engine.objects[item_index].fixed_velocity => FixedVec2::new(expected_force, C4Fixed::ZERO));
    unit_assert_eq!(engine.objects[item_index].state.rotation => 0);
    unit_assert_eq!(engine.objects[item_index].fixed_rotation => C4Fixed::ZERO);
    unit_assert_eq!(engine.objects[item_index].rotation_velocity => C4Fixed::ZERO);
    unit_assert_eq!(engine.objects[item_index].state.velocity.y => 0);
    unit_assert!(engine.objects[item_index].state.mobile);
    unit_assert!(!engine.objects[item_index].state.in_liquid);

    let actor_index = engine.test_object_index(actor_id);
    let actor_state = &engine.objects[actor_index].state;
    unit_assert_eq!(actor_state.local_vars.get("callback_order") => Some(&Value::Int(1234)));
    unit_assert_eq!(actor_state.local_vars.get("ejected") => Some(&object_reference_value(item_id)));
    unit_assert_eq!(actor_state.local_vars.get("departed") => Some(&object_reference_value(item_id)));
    unit_assert_eq!(actor_state.local_vars.get("ungrab_target") => Some(&object_reference_value(target_id)));
    unit_assert_eq!(actor_state.local_vars.get("target_ungrabbed") => Some(&object_reference_value(target_id)));
    unit_assert_eq!(actor_state.local_vars.get("ejection_had_collection") => Some(&Value::Bool(true)));
    unit_assert_eq!(actor_state.local_vars.get("ejection_item_container") => Some(&Value::Nil));
    unit_assert_eq!(actor_state.local_vars.get("ejection_x") => Some(&Value::Int(expected_position.x)));
    unit_assert_eq!(actor_state.local_vars.get("ejection_y") => Some(&Value::Int(expected_position.y)));
    unit_assert_eq!(actor_state.local_vars.get("ejection_xdir") => Some(&Value::Int(math::fixtoi_prec(expected_force, 100))));
    for local in [
        "ejection_ydir",
        "ejection_r",
        "ejection_rdir",
        "ejection_no_collect",
        "departure_no_collect",
    ] {
        unit_assert_eq!(actor_state.local_vars.get(local) => Some(&Value::Int(0)), "{local} observes the pre-delay zeroed Exit state");
    }
    unit_assert_eq!(actor_state.local_vars.get("departure_had_collection") => Some(&Value::Bool(true)));
    unit_assert_eq!(actor_state.local_vars.get("grab_had_collection") => Some(&Value::Bool(false)));
    unit_assert_eq!(actor_state.local_vars.get("grabbed_had_collection") => Some(&Value::Bool(false)));
    unit_assert_eq!(actor_state.no_collect_delay => 2);
    unit_assert_eq!(actor_state.ocf & ocf::COLLECTION => 0);
    unit_assert_eq!(actor_state.action.name => "Walk");
    unit_assert_eq!(actor_state.command_direction => CommandDirection::Stop);
    unit_assert_eq!(engine.objects[actor_index].fixed_velocity => FixedVec2::ZERO);
    Ok(())
}

#[test]
fn object_com_drop_matches_cpp_reduction_thresholds_and_procedure_exceptions(
) -> Result<(), EngineError> {
    // C4ObjectCom.cpp:650-667 uses strict one-sided FIXED10(15)
    // comparisons. Hangle/Swim retain the full edge offset; Scale forces
    // tdir=0 even with a directional ComDir.
    let mut actor = test_definition("DPKM", "Drop kinematics", "#strict 2\n");
    actor.set_shape_rect(Some(DefinitionRect::new(-7, -11, 18, 27)));
    actor.set_physical(PhysicalInfo {
        throw: 50_001,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Hangle".to_string(),
                ActionSpec::default().with_procedure("HANGLE"),
            ),
            (
                "Swim".to_string(),
                ActionSpec::default().with_procedure("SWIM"),
            ),
            (
                "Scale".to_string(),
                ActionSpec::default().with_procedure("SCALE"),
            ),
        ]),
    );
    let mut item = test_definition("DPKI", "Drop matrix item", "#strict 2\n");
    item.set_shape_rect(Some(DefinitionRect::new(-2, -3, 5, 7)));

    let mut engine = seeded_engine_fixture(176, [actor, item]);
    let threshold = math::fixed10(15);
    let force = math::val_by_physical(400, 50_001);
    unit_assert_eq!(threshold.val() => 98_304);
    unit_assert_eq!(force.val() => 131_074);

    let cases = [
        ("Walk", CommandDirection::Left, 98_304, -7, -force),
        ("Walk", CommandDirection::Left, 98_303, 0, -force),
        ("Walk", CommandDirection::Right, -98_304, 11, force),
        ("Walk", CommandDirection::Right, -98_303, 0, force),
        ("Hangle", CommandDirection::UpRight, 0, 11, force),
        ("Swim", CommandDirection::DownLeft, 0, -7, -force),
        ("Scale", CommandDirection::Right, 0, 0, C4Fixed::ZERO),
    ];

    for (case, (action, com_dir, xdir_raw, expected_dx, expected_xdir)) in
        cases.into_iter().enumerate()
    {
        let owner = case as i32 + 1;
        engine.register_player(PlayerConfig::new(owner, format!("Dropper {owner}")))?;
        let actor_id = engine.spawn_object(
            constructed_living_spawn(
                "DPKM",
                100 + case as i32 * 40,
                200,
                ActionState::new(action),
            )
            .with_owner(owner)
            .with_crew_member(true)
            .with_direction(Direction::Left)
            .with_command_direction(com_dir),
        )?;
        let item_id = spawn_full_in(&mut engine, "DPKI", actor_id);
        engine.select_crew(owner, [actor_id])?;
        engine.set_crew_cursor(owner, Some(actor_id))?;
        let actor_index = engine.test_object_index(actor_id);
        engine.objects[actor_index].fixed_velocity.x = C4Fixed::from_raw(xdir_raw);
        engine.objects[actor_index].state.velocity =
            Vector2::new(math::fixtoi(C4Fixed::from_raw(xdir_raw)), 0);
        let actor_position = engine.objects[actor_index].state.position;

        unit_assert!(engine.try_drop_held_object(owner)?);

        let item_index = engine.test_object_index(item_id);
        let expected_position = Vector2::new(actor_position.x + expected_dx, actor_position.y + 12);
        unit_assert_eq!(engine.objects[item_index].state.position => expected_position, "case {case}: {action} {com_dir:?} raw xdir {xdir_raw}");
        unit_assert_eq!(engine.objects[item_index].fixed_position => FixedVec2::from_ints(expected_position.x, expected_position.y), "case {case}: fixed position");
        unit_assert_eq!(engine.objects[item_index].fixed_velocity => FixedVec2::new(expected_xdir, C4Fixed::ZERO), "case {case}: fixed launch");
        unit_assert_eq!(engine.objects[item_index].fixed_rotation => C4Fixed::ZERO);
        unit_assert_eq!(engine.objects[item_index].rotation_velocity => C4Fixed::ZERO);
        unit_assert!(engine.objects[item_index].state.mobile);
        unit_assert!(!engine.objects[item_index].state.in_liquid);
    }
    Ok(())
}

#[test]
fn execute_command_drop_preserves_plain_comdir_and_runs_live_exit() -> Result<(), EngineError> {
    // Untargeted C4Command::Drop calls ObjectComDrop without first
    // writing COMD_Stop (C4Command.cpp:998-1049). ExecuteCommand must
    // therefore preserve the live ComDir through the atomic drop event.
    let actor_script = r#"#strict 2
local callback_order, remove_on_ejection, finished_calls;
local push_on_ejection, preserve_walk, grab_fight_ready, grab_collection;

public func RunDrop()
{
  SetCommand(this(), "Drop");
  ExecuteCommand();
  return(1);
}

public func RunDeletingDrop()
{
  remove_on_ejection = 1;
  SetCommand(this(), "Drop");
  ExecuteCommand();
  return(1);
}

public func RunReenabledDrop()
{
  push_on_ejection = 1;
  preserve_walk = 1;
  SetCommand(this(), "Drop");
  ExecuteCommand();
  return(1);
}

protected func Ejection(pObject)
{
  callback_order = callback_order * 10 + 1;
  if (push_on_ejection) SetAction("DisabledPush", this());
  if (remove_on_ejection) RemoveObject();
  return(1);
}

public func NoteDeparture()
{
  callback_order = callback_order * 10 + 2;
  return(1);
}

protected func Grab(pTarget, fGrab)
{
  if (!fGrab)
  {
    grab_fight_ready = !!(GetOCF() & OCF_FightReady);
    grab_collection = !!(GetOCF() & OCF_Collection);
  }
  return(1);
}

protected func ControlCommandFinished()
{
  finished_calls++;
  if (!preserve_walk) SetAction("Disabled");
  return(1);
}
"#;
    let item_script = r#"#strict 2
protected func Departure(pContainer)
{
  pContainer->NoteDeparture();
  return(1);
}
"#;
    let mut actor = c4_definition("DPCM", "Command dropper", actor_script);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_collection_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_physical(PhysicalInfo {
        throw: 50_000,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Disabled".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_disabled(true),
            ),
            (
                "DisabledPush".to_string(),
                ActionSpec::default()
                    .with_procedure("PUSH")
                    .with_disabled(true),
            ),
        ]),
    );
    let mut item = c4_definition("DPCI", "Command item", item_script);
    item.set_shape_rect(Some(DefinitionRect::new(-3, -3, 6, 6)));

    let mut engine = seeded_engine_fixture(177, [actor, item]);
    let actor_id = engine.spawn_object(
        constructed_living_spawn("DPCM", 100, 200, ActionState::new("Walk"))
            .with_direction(Direction::Left)
            .with_command_direction(CommandDirection::Right),
    )?;
    let item_id = spawn_full_in(&mut engine, "DPCI", actor_id);
    let actor_index = engine.test_object_index(actor_id);
    engine.objects[actor_index].fixed_velocity.x = C4Fixed::from_raw(-98_304);
    engine.objects[actor_index].state.velocity = Vector2::new(-2, 0);
    engine.refresh_object_ocf(actor_index);
    unit_assert_ne!(engine.objects[actor_index].state.ocf & ocf::COLLECTION => 0, "the command preview starts with Collection enabled");
    unit_assert_ne!(engine.objects[actor_index].state.ocf & ocf::FIGHT_READY => 0, "the pre-command Walk actor starts fight-ready");
    let actor_position = engine.objects[actor_index].state.position;
    let item_index = engine.test_object_index(item_id);
    engine.objects[item_index].fixed_velocity = FixedVec2::new(math::itofix(6), -math::itofix(4));
    engine.objects[item_index].state.velocity = Vector2::new(6, -4);
    engine.objects[item_index].state.rotation = 29;
    engine.objects[item_index].fixed_rotation = math::itofix(29);
    engine.objects[item_index].rotation_velocity = math::itofix(3);
    engine.objects[item_index].state.in_liquid = true;

    unit_assert_eq!(engine.call_object_function(actor_index, "RunDrop", Vec::new())? => Value::Int(1));

    let actor_index = engine.test_object_index(actor_id);
    unit_assert_eq!(engine.objects[actor_index].state.command_direction => CommandDirection::Right);
    unit_assert_eq!(engine.objects[actor_index].state.local_vars.get("callback_order") => Some(&Value::Int(12)));
    unit_assert_eq!(engine.objects[actor_index].state.no_collect_delay => 2);
    unit_assert_eq!(engine.objects[actor_index].state.ocf & ocf::COLLECTION => 0);
    unit_assert_eq!(engine.objects[actor_index].state.action.name => "Disabled");
    unit_assert_eq!(engine.objects[actor_index].state.ocf & ocf::FIGHT_READY => 0, "final OCF includes ControlCommandFinished's disabled action");
    let item_index = engine.test_object_index(item_id);
    unit_assert_eq!(engine.objects[item_index].state.container => None);
    unit_assert_eq!(engine.objects[item_index].state.position => Vector2::new(actor_position.x + 8, actor_position.y + 7));
    unit_assert_eq!(engine.objects[item_index].fixed_velocity => FixedVec2::new(math::val_by_physical(400, 50_000), C4Fixed::ZERO));
    unit_assert_eq!(engine.objects[item_index].state.rotation => 0);
    unit_assert_eq!(engine.objects[item_index].fixed_rotation => C4Fixed::ZERO);
    unit_assert_eq!(engine.objects[item_index].rotation_velocity => C4Fixed::ZERO);
    unit_assert_eq!(engine.objects[item_index].state.velocity.y => 0);
    unit_assert!(!engine.objects[item_index].state.in_liquid);

    // Removing the dropper during Ejection gives it raw Status zero;
    // C4Object::Call must suppress the later ControlCommandFinished.
    let deleted_actor = engine.spawn_object(
        constructed_living_spawn("DPCM", 300, 200, ActionState::new("Walk"))
            .with_command_direction(CommandDirection::Right),
    )?;
    let deleted_item = spawn_full_in(&mut engine, "DPCI", deleted_actor);
    let deleted_index = engine.test_object_index(deleted_actor);
    unit_assert_eq!(engine.call_object_function(deleted_index, "RunDeletingDrop", Vec::new())? => Value::Int(1));
    let deleted_index = engine.test_object_index(deleted_actor);
    unit_assert_eq!(engine.objects[deleted_index].state.status => ObjectStatus::Deleted);
    unit_assert_ne!(
        engine.objects[deleted_index]
            .state
            .local_vars
            .get("finished_calls") =>
        Some(&Value::Int(1)),
        "deleted actors receive no ControlCommandFinished callback"
    );
    let deleted_item_index = engine.test_object_index(deleted_item);
    unit_assert_eq!(engine.objects[deleted_item_index].state.container => None);

    // SetActionByName("Walk") inside ObjectComUnGrab runs SetOCF before
    // Grab(false). It must restore FightReady after an Ejection callback
    // temporarily installs an ObjectDisabled Push action, while the
    // adjacent delay=2 assignment keeps Collection off.
    let reenabled_actor = engine.spawn_object(constructed_living_spawn(
        "DPCM",
        500,
        200,
        ActionState::new("Walk"),
    ))?;
    let _reenabled_item = spawn_full_in(&mut engine, "DPCI", reenabled_actor);
    let reenabled_index = engine.test_object_index(reenabled_actor);
    engine.refresh_object_ocf(reenabled_index);
    unit_assert_ne!(engine.objects[reenabled_index].state.ocf & ocf::FIGHT_READY => 0);
    unit_assert_eq!(engine.call_object_function(reenabled_index, "RunReenabledDrop", Vec::new())? => Value::Int(1));
    let reenabled_index = engine.test_object_index(reenabled_actor);
    let reenabled = &engine.objects[reenabled_index].state;
    unit_assert_eq!(reenabled.local_vars.get("grab_fight_ready") => Some(&Value::Bool(true)));
    unit_assert_eq!(reenabled.local_vars.get("grab_collection") => Some(&Value::Bool(false)));
    unit_assert_eq!(reenabled.action.name => "Walk");
    unit_assert_eq!(reenabled.no_collect_delay => 2);
    unit_assert_ne!(reenabled.ocf & ocf::FIGHT_READY => 0);
    unit_assert_eq!(reenabled.ocf & ocf::COLLECTION => 0);
    Ok(())
}

/// CanBeBase hut plus FLAG's FlyBase action (Flag.c4d ActMap) for ExecBase tests.
fn base_fixture() -> Result<(Engine, ObjectId, ObjectId), EngineError> {
    let mut hut = c4_definition(
        "HUT1",
        "Hut",
        r#"#strict 2
        protected func Ejection(object flag)
        {
          flag->RecordExecBaseEjection();
          return(1);
        }
        "#,
    );
    hut.set_can_be_base(true);
    let mut flag = c4_definition(
        "FLAG",
        "Flag",
        r#"#strict 2
        local callback_order;
        local ejection_x, ejection_y, ejection_r, ejection_xdir, ejection_ydir, ejection_rdir;
        local ejection_container, ejection_action;
        local departure_x, departure_y, departure_r, departure_xdir, departure_ydir, departure_rdir;
        local departure_container, departure_action;
        local flyBaseStartTarget, flyBaseStartAction, flyBaseStartOwner;

        public func RecordExecBaseEjection()
        {
          var no_object;
          callback_order = callback_order * 10 + 1;
          ejection_x = GetX();
          ejection_y = GetY();
          ejection_r = GetR();
          ejection_xdir = GetXDir(no_object, 100);
          ejection_ydir = GetYDir(no_object, 100);
          ejection_rdir = GetRDir(no_object, 100);
          ejection_container = Contained();
          ejection_action = GetAction();
          return(1);
        }

        protected func Departure(object old_container)
        {
          var no_object;
          callback_order = callback_order * 10 + 2;
          departure_x = GetX();
          departure_y = GetY();
          departure_r = GetR();
          departure_xdir = GetXDir(no_object, 100);
          departure_ydir = GetYDir(no_object, 100);
          departure_rdir = GetRDir(no_object, 100);
          departure_container = Contained();
          departure_action = GetAction();
          return(1);
        }

        protected func FlyBaseStart()
        {
          callback_order = callback_order * 10 + 3;
          flyBaseStartTarget = GetActionTarget();
          flyBaseStartAction = GetAction();
          if (flyBaseStartOwner) SetOwner(flyBaseStartOwner);
          return(1);
        }
        "#,
    );
    let mut actions = HashMap::new();
    actions.insert(
        "FlyBase".to_string(),
        ActionSpec::default()
            .with_procedure("attach")
            .with_start_call("FlyBaseStart"),
    );
    flag.configure_actions(None, actions);
    let mut engine = engine_fixture([hut, flag]);
    engine.register_player(PlayerConfig::new(1, "Test"))?;
    let hut = engine.spawn_object(SpawnConfig::new("HUT1"))?;
    let flag = engine.spawn_object(
        SpawnConfig::new("FLAG")
            .with_owner(1)
            .with_position(Vector2::new(17, 29))
            .with_fixed_velocity(FixedVec2::new(itofix(3), itofix(-2)))
            .with_rotation(19)
            .with_fixed_rotation(itofix(19))
            .with_rotation_velocity(itofix(4))
            .with_in_liquid(true)
            .with_mobile(false)
            .with_container(hut),
    )?;
    Ok((engine, hut, flag))
}

fn auto_sell_enter_engine() -> Result<Engine, EngineError> {
    let mut engine = Engine::new();
    engine.register_player(PlayerConfig::new(1, "Test"))?;
    let mut crew = c4_definition(
        "CLNK",
        "Clonk",
        r#"#strict
        local entrance_count;

        public func Board(pTarget)
        {
          return(SetCommand(this(), "Enter", pTarget));
        }

        protected func Entrance(pTarget)
        {
          entrance_count += 1;
          return(1);
        }
        "#,
    );
    crew.set_crew_member(true);
    engine.register_definition(crew)?;

    let mut gold = test_definition("GOLD", "Gold", BASIC_OBJECT_SCRIPT);
    gold.set_value(5);
    gold.set_base_auto_sell(true);
    gold.set_rebuyable(false);
    engine.register_definition(gold)?;
    Ok(engine)
}

fn enterable_base_definition(id: &str, script: &str) -> Result<Definition, EngineError> {
    let mut base = c4_definition(id, id, script);
    base.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    base.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    Ok(base)
}

fn activate_test_base(
    engine: &mut Engine,
    definition_id: &str,
    position: Vector2,
    owner: i32,
) -> Result<ObjectId, EngineError> {
    let base = engine.spawn_object(
        SpawnConfig::new(definition_id)
            .with_category(CATEGORY_STRUCTURE)
            .with_position(position),
    )?;
    let base_index = engine
        .find_object_index(base)
        .ok_or(EngineError::UnknownObject(base))?;
    engine.objects[base_index].state.base = owner;
    engine.objects[base_index].state.entrance_status = true;
    engine.refresh_object_ocf(base_index);
    Ok(base)
}

fn queue_enter_command(
    engine: &mut Engine,
    crew: ObjectId,
    target: ObjectId,
) -> Result<(), EngineError> {
    let crew_index = engine
        .find_object_index(crew)
        .ok_or(EngineError::UnknownObject(crew))?;
    unit_assert_eq!(engine.call_object_function(crew_index, "Board", vec![object_reference_value(target)],)? => Value::Bool(true));
    Ok(())
}

#[test]
fn exec_base_tick35_digs_snow_and_fly_ashes_out_of_upright_structures() -> Result<(), EngineError> {
    // C4Object::ExecBase clears exactly Snow and FlyAshes from an
    // upright structure's current shape rectangle on Tick35 when the
    // StructuresSnowIn rule is absent (C4Object.cpp:1033-1044).
    let library = MaterialLibrary::parse(
        r#"
            [Material Snow]
            Name=Snow
            Density=25
            DigFree=1

            [Material FlyAshes]
            Name=FlyAshes
            Density=25
            DigFree=1

            [Material Earth]
            Name=Earth
            Density=100
            DigFree=0
            "#,
    )
    .test_value();
    let materials = MaterialSet::from_resource_library(&library);
    let snow = materials.id_of("Snow").test_value();
    let fly_ashes = materials.id_of("FlyAshes").test_value();
    let earth = materials.id_of("Earth").test_value();
    unit_assert!(materials.get_by_id(snow).unwrap().dig_free());
    unit_assert!(materials.get_by_id(fly_ashes).unwrap().dig_free());

    let mut engine = Engine::new();
    engine.set_materials(materials);
    let mut structure = test_definition("HUT1", "Hut", BASIC_OBJECT_SCRIPT);
    structure.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
    engine.register_definition(structure)?;

    let mut bytes = vec![0_u8; 7 * 7];
    bytes[2 * 7 + 2] = 10;
    bytes[3 * 7 + 4] = 20;
    bytes[3 * 7 + 3] = 30;
    let mut densities = vec![0_i32; 128];
    densities[10] = 25;
    densities[20] = 25;
    densities[30] = 100;
    let mut names = vec![None; 128];
    names[10] = Some("Snow".to_string());
    names[20] = Some("FlyAshes".to_string());
    names[30] = Some("Earth".to_string());
    let grid = landscape::PixelGrid::new(7, 7, bytes, densities, names, vec![None; 128]);
    let mut landscape = Landscape::new(7, vec![7; 7]).test_value();
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    let structure = engine.spawn_object(
        SpawnConfig::new("HUT1")
            .with_position(Vector2::new(3, 3))
            .with_category(CATEGORY_STRUCTURE),
    )?;
    advance_fixture(&mut engine, 34)?;
    let structure_index = engine.test_object_index(structure);
    engine.objects[structure_index].fixed_rotation = C4Fixed::from_raw(1);
    unit_assert_eq!(engine.objects[structure_index].state.rotation => 0);
    unit_assert_ne!(engine.objects[structure_index].fixed_rotation => C4Fixed::ZERO, "sub-degree fix_r distinguishes the integer-r C++ gate");
    unit_assert_eq!(engine.landscape().unwrap().material_at(2, 2) => Some(snow));
    unit_assert_eq!(engine.landscape().unwrap().material_at(4, 3) => Some(fly_ashes));

    engine.tick_without_snapshot()?;
    unit_assert_eq!(engine.landscape().unwrap().material_at(2, 2) => None);
    unit_assert_eq!(engine.landscape().unwrap().material_at(4, 3) => None);
    unit_assert_eq!(engine.landscape().unwrap().material_at(3, 3) => Some(earth), "DigFreeMat only targets Snow and FlyAshes");
    Ok(())
}

#[test]
fn structures_snow_in_rule_preserves_snow_inside_structures() -> Result<(), EngineError> {
    // C4Game::UpdateRules maps a live STSN object to
    // C4RULE_StructuresSnowIn (C4Game.cpp:4038-4047), which suppresses
    // ExecBase's Tick35 DigFreeMat calls (C4Object.cpp:1033-1044).
    let library = MaterialLibrary::parse(
        r#"
            [Material Snow]
            Name=Snow
            Density=25
            DigFree=1
            "#,
    )
    .test_value();
    let materials = MaterialSet::from_resource_library(&library);
    let snow = materials.id_of("Snow").test_value();
    let mut engine = Engine::new();
    engine.set_materials(materials.clone());

    let mut structure = test_definition("HUT1", "Hut", BASIC_OBJECT_SCRIPT);
    structure.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
    engine.register_definition(structure)?;
    engine.register_test_script_definition("STSN", "Structures Snow In", BASIC_OBJECT_SCRIPT);

    let mut bytes = vec![0_u8; 7 * 7];
    bytes[2 * 7 + 2] = 10;
    let mut densities = vec![0_i32; 128];
    densities[10] = 25;
    let mut names = vec![None; 128];
    names[10] = Some("Snow".to_string());
    let grid = landscape::PixelGrid::new(7, 7, bytes, densities, names, vec![None; 128]);
    let mut landscape = Landscape::new(7, vec![7; 7]).test_value();
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    engine.spawn_object(
        SpawnConfig::new("HUT1")
            .with_position(Vector2::new(3, 3))
            .with_category(CATEGORY_STRUCTURE),
    )?;
    engine.spawn_object(SpawnConfig::new("STSN"))?;

    advance_fixture(&mut engine, 34)?;
    let saved = engine.capture_state();

    let mut restored = Engine::new();
    restored.set_materials(materials);
    let mut structure = test_definition("HUT1", "Hut", BASIC_OBJECT_SCRIPT);
    structure.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
    restored.register_definition(structure)?;
    restored.register_test_script_definition("STSN", "Structures Snow In", BASIC_OBJECT_SCRIPT);
    restored.restore_state(&saved)?;
    restored.tick_without_snapshot()?;
    unit_assert_eq!(restored.landscape().unwrap().material_at(2, 2) => Some(snow), "the saved STSN rule bit disables frame-35 structure snow clearing");
    Ok(())
}

#[test]
fn exec_base_exit_zeroes_motion_and_runs_callbacks_before_flybase_start() -> Result<(), EngineError>
{
    // ExecBase calls the default-argument Exit(0,0) before FlyBase
    // (C4Object.cpp:1010-1012). Exit publishes its zeroed transform and
    // motion before Ejection/Departure (C4Object.cpp:1532-1564), then
    // SetAction installs the base target before FlyBase's StartCall
    // (C4Object.cpp:4148-4184).
    let (mut engine, hut, flag) = base_fixture()?;
    advance_fixture(&mut engine, 11)?;

    let flag_index = engine.test_object_index(flag);
    let flag_state = &engine.objects[flag_index].state;
    unit_assert_eq!(flag_state.local_vars.get("callback_order") => Some(&Value::Int(123)), "Ejection and Departure precede FlyBase's StartCall");
    for local in [
        "ejection_x",
        "ejection_y",
        "ejection_r",
        "ejection_xdir",
        "ejection_ydir",
        "ejection_rdir",
        "departure_x",
        "departure_y",
        "departure_r",
        "departure_xdir",
        "departure_ydir",
        "departure_rdir",
    ] {
        unit_assert_eq!(flag_state.local_vars.get(local) => Some(&Value::Int(0)), "{local} observes Exit's default zero transform/motion");
    }
    for local in ["ejection_container", "departure_container"] {
        unit_assert_eq!(flag_state.local_vars.get(local) => Some(&Value::Nil), "{local} observes the completed unlink");
    }
    for local in ["ejection_action", "departure_action"] {
        unit_assert_eq!(flag_state.local_vars.get(local) => Some(&Value::String("Idle".to_string().into())), "{local} runs before FlyBase");
    }
    unit_assert_eq!(flag_state.local_vars.get("flyBaseStartTarget") => Some(&Value::Object(hut.as_u64())), "FlyBase StartCall observes the supplied base target");
    unit_assert_eq!(flag_state.local_vars.get("flyBaseStartAction") => Some(&Value::String("FlyBase".to_string().into())));
    Ok(())
}

#[test]
fn exec_base_uses_the_flag_owner_after_flybase_callbacks() -> Result<(), EngineError> {
    // The guard reads flag->Owner before Exit, but Base/SetOwner read it
    // again after Exit and FlyBase callbacks (C4Object.cpp:1008-1018).
    let (mut engine, hut, flag) = base_fixture()?;
    engine.register_player(PlayerConfig::new(2, "Callback owner"))?;
    let flag_index = engine.test_object_index(flag);
    engine.objects[flag_index]
        .state
        .local_vars
        .insert("flyBaseStartOwner".to_string(), Value::Int(2));

    advance_fixture(&mut engine, 11)?;

    let hut_index = engine.test_object_index(hut);
    unit_assert_eq!(engine.objects[hut_index].state.base => 2);
    unit_assert_eq!(engine.objects[hut_index].state.owner => 2);
    unit_assert_eq!(engine.object_snapshot(flag).expect("flag remains").owner => 2);
    Ok(())
}

#[test]
fn exec_base_assigns_base_from_a_contained_flag_like_cpp() -> Result<(), EngineError> {
    // ExecBase's Tick10 arm (C4Object.cpp:1000-1018): a CanBeBase
    // object without a valid base that contains a flag of a valid
    // player exits the flag onto FlyBase, becomes that player's base
    // and takes the flag's owner.
    let (mut engine, hut, flag) = base_fixture()?;

    let mut audio = Vec::new();
    for _ in 0..11 {
        audio.extend(engine.tick()?.audio);
    }
    let hut_index = engine.test_object_index(hut);
    unit_assert_eq!(engine.objects[hut_index].state.base => 1, "Base = flag->Owner (C4Object.cpp:1014)");
    unit_assert_eq!(engine.objects[hut_index].state.owner => 1, "SetOwner(flag->Owner) (C4Object.cpp:1018)");
    let flag_index = engine.test_object_index(flag);
    unit_assert_eq!(engine.objects[flag_index].state.container => None, "flag->Exit() (C4Object.cpp:1010)");
    unit_assert_eq!(engine.objects[flag_index].state.action.name => "FlyBase", "flag->SetActionByName(\"FlyBase\", this) (C4Object.cpp:1011)");
    unit_assert_eq!(engine.objects[flag_index].state.action.target => Some(hut));
    unit_assert!(
        audio.iter().any(|command| matches!(
            command,
            AudioCommand::PlaySound { name, target, .. }
                if name == "Trumpet" && *target == Some(hut)
        )),
        "the Trumpet fanfare plays at the new base (C4Object.cpp:1017)"
    );
    Ok(())
}

#[test]
fn exec_base_clears_base_when_the_flag_is_lost() -> Result<(), EngineError> {
    // ExecBase's Tick35 arm (C4Object.cpp:1024-1031): a valid base
    // without a FlyBase flag targeting it loses the base assignment.
    let (mut engine, hut, flag) = base_fixture()?;
    advance_fixture(&mut engine, 11)?;
    let hut_index = engine.test_object_index(hut);
    unit_assert_eq!(engine.objects[hut_index].state.base => 1);
    let occupant = spawn_fixture!(engine, "FLAG", with_container: hut);
    engine.apply_object_update(
        occupant,
        ObjectUpdate {
            menu: Some(Some(ObjectMenuState {
                caption: "Base".to_string(),
                symbol_id: "HUT1".to_string(),
                title_symbol: ObjectMenuSymbol::default(),
                identification: Value::Int(14),
                style: 1,
                equal_item_height: false,
                permanent: true,
                location: None,
                runtime_id: 0,
                extra: ObjectMenuExtra::default(),
                extra_data: 0,
                internal_refill_token: 0,
                selection: -1,
                user_menu: false,
                command_object: Some(occupant),
                scenario_callbacks: false,
                refill_object: None,
                refill_object_contents_count: 0,
                location_reset_generation: 0,
                items: Vec::new(),
                columns: 1,
                lines: 0,
                text_progressing: false,
                decoration: None,
            })),
            ..ObjectUpdate::default()
        },
    )?;

    engine.apply_object_update(flag, ObjectUpdate::new().with_status(ObjectStatus::Deleted))?;
    advance_fixture(&mut engine, 36)?;
    let hut_index = engine.test_object_index(hut);
    unit_assert_eq!(engine.objects[hut_index].state.base => OWNER_NONE, "lost flag clears Base (C4Object.cpp:1027-1030)");
    unit_assert_eq!(
        engine.debug_object_menu(occupant.as_u64()) =>
        Some(None),
        "lost flag closes every contained menu (C4Object.cpp:1029; C4ObjectList.cpp:705-710)"
    );
    Ok(())
}

#[test]
fn exec_base_auto_sells_nested_gold_like_cpp() -> Result<(), EngineError> {
    // C4Object::ExecBase calls AutoSellContents on Tick35 when the
    // BASEFUNC_AutoSellContents bit is active. It walks each base
    // occupant's contents, exits BaseAutoSell items, and passes them to
    // C4Player::Sell2Home (src/C4Object.cpp:970-995,1021-1029;
    // src/C4Player.cpp:865-897).
    let (mut engine, hut, _flag) = base_fixture()?;

    let mut crew = test_definition("CLNK", "Clonk", BASIC_OBJECT_SCRIPT);
    crew.set_crew_member(true);
    engine.register_definition(crew)?;

    let mut gold = test_definition("GOLD", "Gold", BASIC_OBJECT_SCRIPT);
    gold.set_value(5);
    gold.set_base_auto_sell(true);
    gold.set_rebuyable(false);
    engine.register_definition(gold)?;

    let crew = engine.spawn_object(
        SpawnConfig::new("CLNK")
            .with_owner(1)
            .with_alive(true)
            .with_crew_member(true)
            .with_container(hut),
    )?;
    let gold = spawn_fixture!(engine, "GOLD", with_container: crew);

    advance_fixture(&mut engine, 36)?;

    unit_assert_eq!(engine.player(1).expect("player exists").wealth() => 5);
    unit_assert!(engine.object_snapshot(gold).is_none(), "Sell2Home removes the sold GOLD");
    unit_assert!(!engine.player(1).expect("player exists").home_base_material().contains_key("GOLD"), "Rebuy=0 does not introduce a missing home-base stock entry");
    Ok(())
}

#[test]
fn auto_sell_uses_calc_value_sell_to_and_sale_like_cpp() -> Result<(), EngineError> {
    // Sell2Home gets the value first, then lets SellTo remap the stock ID,
    // updates stock, invokes Sale(player), and finally removes the sold
    // object (src/C4Player.cpp:876-897). AutoSellContents exits it before
    // this transaction, so CalcValue receives nil for pInBase
    // (src/C4Object.cpp:984-987; C4Object.cpp:2118-2144).
    let (mut engine, hut, _flag) = base_fixture()?;

    let mut crew = test_definition("CLNK", "Clonk", BASIC_OBJECT_SCRIPT);
    crew.set_crew_member(true);
    engine.register_definition(crew)?;

    let mut remapped = test_definition("RMAP", "Remapped", BASIC_OBJECT_SCRIPT);
    remapped.set_rebuyable(true);
    engine.register_definition(remapped)?;
    engine.register_test_script_definition("SALE", "Sale marker", BASIC_OBJECT_SCRIPT);

    let mut sold = test_definition(
        "AUTO",
        "Auto-sold",
        r#"#strict
        protected func CalcValue(object pInBase, int player)
        {
          if (pInBase) return(99);
          return(7);
        }
        protected func SellTo(int player) { return(RMAP); }
        protected func Sale(int player)
        {
          if (GetWealth(player) == 7)
            if (GetHomebaseMaterial(player, RMAP) == 1)
              CreateObject(SALE, 0, 0, player);
          return(1);
        }
        "#,
    );
    sold.set_value(50);
    sold.set_base_auto_sell(true);
    engine.register_definition(sold)?;

    let crew = engine.spawn_object(
        SpawnConfig::new("CLNK")
            .with_owner(1)
            .with_alive(true)
            .with_crew_member(true)
            .with_container(hut),
    )?;
    let sold = spawn_fixture!(engine, "AUTO", with_container: crew);

    advance_fixture(&mut engine, 36)?;

    let player = engine.player(1).test_value();
    unit_assert_eq!(player.wealth() => 7, "CalcValue overrides static Value=50");
    unit_assert_eq!(player.home_base_material().get("RMAP") => Some(&1), "SellTo remaps stock before Sale observes it");
    unit_assert!(
        engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "SALE" && object.owner == 1),
        "Sale(player) runs after wealth and stock update"
    );
    unit_assert!(engine.object_snapshot(sold).is_none());
    Ok(())
}

#[test]
fn command_enter_auto_sells_base_contents_synchronously_like_cpp() -> Result<(), EngineError> {
    // C4Object::Enter invokes Contained->AutoSellContents immediately
    // after Collection2 and Entrance; it does not wait for ExecBase's
    // Tick35 pass (src/C4Object.cpp:1625-1634,970-995).
    let mut engine = auto_sell_enter_engine()?;
    engine.register_definition(enterable_base_definition("BASE", "#strict\n")?)?;

    let base = activate_test_base(&mut engine, "BASE", Vector2::new(100, 120), 1)?;
    let crew = spawn_crew_at_fixture(&mut engine, "CLNK", 1, 100, 100);
    let gold = spawn_fixture!(engine, "GOLD", with_container: crew);
    queue_enter_command(&mut engine, crew, base)?;

    engine.tick_without_snapshot()?;

    let crew_index = engine.test_object_index(crew);
    unit_assert_eq!(engine.objects[crew_index].state.container => Some(base));
    unit_assert_eq!(engine.player(1).expect("player exists").wealth() => 5, "the Enter frame must include the GOLD sale");
    unit_assert!(engine.object_snapshot(gold).is_none(), "the synchronously sold GOLD is removed");
    Ok(())
}

#[test]
fn nested_script_enter_finishes_before_the_outer_target_liveness_gate_like_cpp(
) -> Result<(), EngineError> {
    // Collection2 may move the entrant with a nested script Enter. That
    // nested Enter finishes its own Entrance and auto-sale before the
    // callback returns. The outer Entrance/auto-sale tail then uses the
    // current Contained, but still requires its original pTarget to be
    // live (src/C4Object.cpp:1625-1634).
    fn run_case(
        remove_original: bool,
    ) -> Result<(Engine, ObjectId, ObjectId, ObjectId), EngineError> {
        let mut engine = auto_sell_enter_engine()?;
        engine.register_definition(enterable_base_definition("DEST", "#strict\n")?)?;
        engine.register_definition(enterable_base_definition(
            "TARG",
            r#"#strict
local destination, removeOriginal;
protected func Collection2(pObject)
{
  Enter(destination, pObject);
  if (removeOriginal) RemoveObject();
  return(1);
}
"#,
        )?)?;

        let destination = activate_test_base(&mut engine, "DEST", Vector2::new(300, 120), 1)?;
        let target = engine.spawn_object(
            SpawnConfig::new("TARG")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(100, 120))
                .with_local_vars(HashMap::from([
                    (
                        "destination".to_string(),
                        object_reference_value(destination),
                    ),
                    (
                        "removeOriginal".to_string(),
                        Value::Int(i32::from(remove_original)),
                    ),
                ])),
        )?;
        let target_index = engine.test_object_index(target);
        engine.objects[target_index].state.entrance_status = true;
        engine.refresh_object_ocf(target_index);

        let crew = spawn_crew_at_fixture(&mut engine, "CLNK", 1, 100, 100);
        let gold = spawn_fixture!(engine, "GOLD", with_container: crew);
        queue_enter_command(&mut engine, crew, target)?;
        engine.tick_without_snapshot()?;

        let crew_index = engine.test_object_index(crew);
        unit_assert_eq!(engine.objects[crew_index].state.container => Some(destination), "Collection2 redirected the entrant before the callback tail");
        Ok((engine, crew, gold, target))
    }

    let (live_target, live_crew, live_gold, _target) = run_case(false)?;
    unit_assert_eq!(live_target.player(1).expect("player exists").wealth() => 5, "the nested Enter auto-sells in the redirected base");
    unit_assert!(live_target.object_snapshot(live_gold).is_none());
    let live_crew_index = live_target.test_object_index(live_crew);
    unit_assert_eq!(
        live_target.objects[live_crew_index]
            .state
            .local_vars
            .get("entrance_count") =>
        Some(&Value::Int(2)),
        "nested and outer Enter both reach their Entrance tail"
    );

    let (removed_target, removed_crew, sold_gold, target) = run_case(true)?;
    unit_assert!(removed_target.object_snapshot(target).is_none(), "Collection2 removed the original target");
    unit_assert_eq!(removed_target.player(1).expect("player exists").wealth() => 5, "the nested Enter sells before Collection2 removes the outer pTarget");
    unit_assert!(removed_target.object_snapshot(sold_gold).is_none(), "the nested Enter completed its auto-sale");
    let removed_crew_index = removed_target.test_object_index(removed_crew);
    unit_assert_eq!(
        removed_target.objects[removed_crew_index]
            .state
            .local_vars
            .get("entrance_count") =>
        Some(&Value::Int(1)),
        "a removed original pTarget suppresses only the outer Entrance tail"
    );
    Ok(())
}

#[test]
fn command_enter_skips_auto_sell_when_base_functionality_is_disabled_like_cpp(
) -> Result<(), EngineError> {
    // C4Object::Enter gates its synchronous AutoSellContents call on
    // BASEFUNC_AutoSellContents (src/C4Object.cpp:1631-1634).
    let mut engine = auto_sell_enter_engine()?;
    engine.set_base_auto_sell_enabled(false);
    engine.register_definition(enterable_base_definition("BASE", "#strict\n")?)?;

    let base = activate_test_base(&mut engine, "BASE", Vector2::new(100, 120), 1)?;
    let crew = spawn_crew_at_fixture(&mut engine, "CLNK", 1, 100, 100);
    let gold = spawn_fixture!(engine, "GOLD", with_container: crew);
    queue_enter_command(&mut engine, crew, base)?;

    engine.tick_without_snapshot()?;

    unit_assert_eq!(engine.player(1).expect("player exists").wealth() => 0);
    let gold_index = engine.test_object_index(gold);
    unit_assert_eq!(engine.objects[gold_index].state.container => Some(crew));
    Ok(())
}

fn flag_collection_fixture(
    flag_removeable: bool,
) -> Result<(Engine, ObjectId, ObjectId), EngineError> {
    let collector_script = r#"#strict
local reject_collect_calls, collection_calls;
public func Take(pItem) { return Collect(pItem); }
protected func RejectCollect(idItem, pItem)
{
  reject_collect_calls = reject_collect_calls + 1;
  return 0;
}
protected func Collection(pItem)
{
  collection_calls = collection_calls + 1;
  return 1;
}
"#;
    let flag_script = r#"#strict
local reject_entrance_calls, entrance_calls;
protected func RejectEntrance(pTarget)
{
  reject_entrance_calls = reject_entrance_calls + 1;
  return 0;
}
protected func Entrance(pTarget)
{
  entrance_calls = entrance_calls + 1;
  return 1;
}
"#;

    let mut collector = c4_definition("FLCN", "Flag collector", collector_script);
    collector.set_shape_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    collector.set_collection_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    let mut flag = c4_definition("FLAG", "Flag", flag_script);
    flag.set_collectible(true);
    flag.configure_actions(
        Some("Idle".to_string()),
        HashMap::from([
            ("Idle".to_string(), ActionSpec::default()),
            (
                "FlyBase".to_string(),
                ActionSpec::default().with_procedure("ATTACH"),
            ),
        ]),
    );

    let mut engine = engine_fixture([collector, flag, simple_definition("FGRV")]);
    if flag_removeable {
        engine.spawn_object(SpawnConfig::new("FGRV"))?;
        // C4Game::InitRules calls UpdateRules before play; the runtime
        // refresh repeats at frame one and every Tick255.
        engine.tick_without_snapshot()?;
    }
    let collector = spawn_living_at_fixture(&mut engine, "FLCN", 100, 100);
    let flag = engine.spawn_object(
        SpawnConfig::new("FLAG")
            .with_action(ActionState::new("FlyBase"))
            .with_position(Vector2::new(100, 95)),
    )?;
    Ok((engine, collector, flag))
}

#[test]
fn script_collect_blocks_flybase_flag_unless_flag_removeable_rule() -> Result<(), EngineError> {
    let (mut blocked, collector, flag) = flag_collection_fixture(false)?;
    let flag_before = blocked.test_object_snapshot(flag);
    let collector_idx = blocked.test_object_index(collector);

    unit_assert_eq!(blocked.call_object_function(collector_idx, "Take", vec![object_reference_value(flag)],)? => Value::Bool(false));
    let blocked_collector = blocked.test_object_snapshot(collector);
    unit_assert!(blocked_collector.contents.is_empty());
    unit_assert_eq!(blocked_collector.local_vars.get("reject_collect_calls") => Some(&Value::Nil), "the special gate precedes RejectCollect");
    unit_assert_eq!(blocked_collector.local_vars.get("collection_calls") => Some(&Value::Nil), "the special gate precedes Collection");
    unit_assert_eq!(blocked.object_snapshot(flag) => Some(flag_before), "the special gate precedes RejectEntrance and attach cancellation");

    let (mut allowed, collector, flag) = flag_collection_fixture(true)?;
    let collector_idx = allowed.test_object_index(collector);
    unit_assert_eq!(allowed.call_object_function(collector_idx, "Take", vec![object_reference_value(flag)],)? => Value::Bool(true));
    unit_assert_eq!(allowed.object_snapshot(flag).expect("flag remains").container => Some(collector));
    Ok(())
}

#[test]
fn cross_check_blocks_flybase_flag_unless_flag_removeable_rule() -> Result<(), EngineError> {
    let (mut blocked, collector, flag) = flag_collection_fixture(false)?;
    let collector_before = blocked.test_object_snapshot(collector);
    let flag_before = blocked.test_object_snapshot(flag);

    blocked.cross_check(3)?;
    unit_assert_eq!(blocked.object_snapshot(collector) => Some(collector_before), "blocked auto-collection calls no collector callback");
    unit_assert_eq!(blocked.object_snapshot(flag) => Some(flag_before), "blocked auto-collection leaves the FlyBase flag exact");

    let (mut allowed, collector, flag) = flag_collection_fixture(true)?;
    allowed.cross_check(3)?;
    unit_assert_eq!(allowed.object_snapshot(flag).expect("flag remains").container => Some(collector));
    Ok(())
}

#[test]
fn auto_collect_moves_carryable_into_inventory() -> Result<(), EngineError> {
    // Bottom-anchor the crew at center y=0 (C4Object.cpp:1462-1468).
    let (mut engine, crew, [item]) = auto_collect_fixture(None, [2]);

    // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
    advance_fixture(&mut engine, 3)?;

    let item_snapshot = engine.test_object_snapshot(item);
    unit_assert_eq!(item_snapshot.container => Some(crew));
    Ok(())
}

#[test]
fn cross_check_collection_rereads_item_position_after_query_catch_blow() -> Result<(), EngineError>
{
    // A truthy QueryCatchBlow falls through to collection, which rereads the
    // live object positions before testing the collection rectangle
    // (C4GameObjects.cpp:167-190). Moving the item out during the callback
    // must therefore prevent collection.
    let mut collector = c4_definition(
        "COLL",
        "Collector",
        "#strict 2\nfunc QueryCatchBlow(item) { SetPosition(90, 50, item); return 1; }\n",
    );
    collector.set_category(CATEGORY_LIVING);
    collector.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    collector.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    let mut item = test_definition("ITEM", "Item", "#strict 2\n");
    item.set_category(CATEGORY_OBJECT);
    item.set_collectible(true);
    let mut engine = engine_fixture([collector, item]);

    let collector = spawn_fixture!(engine, "COLL", with_loaded: true, with_alive: true, with_construction: FULL_CON, with_position: Vector2::new(50, 50));
    let item = spawn_fixture!(engine, "ITEM", with_loaded: true, with_construction: FULL_CON, with_position: Vector2::new(50, 50), with_velocity: Vector2::new(5, 0));

    engine.cross_check(3)?;

    let item = engine.test_object_snapshot(item);
    unit_assert_eq!(item.container => None);
    unit_assert_eq!(item.position => Vector2::new(90, 50));
    unit_assert!(engine.test_object_snapshot(collector).contents.is_empty());
    Ok(())
}

#[test]
fn cross_check_collection_uses_live_rectangle_after_query_changedef(
) -> Result<(), EngineError> {
    // QueryCatchBlow runs before CrossCheck's collection arm. A truthy result
    // skips the Hit body, then C++ rereads obj1->Def->Collection
    // (C4GameObjects.cpp:167-190, especially :186-188).
    // ChangeDef swaps Def and refreshes OCF before returning to the callback
    // (C4Object.cpp:1223-1245).
    let mut narrow = c4_definition(
        "NARR",
        "Narrow collector",
        r#"#strict 2
protected func QueryCatchBlow(object item)
{
    ChangeDef(WIDE);
    return(1);
}
"#,
    );
    narrow.set_category(CATEGORY_LIVING);
    narrow.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    narrow.set_collection_rect(Some(DefinitionRect::new(-2, -2, 4, 4)));

    let mut wide = c4_definition("WIDE", "Wide collector", "#strict 2\n");
    wide.set_category(CATEGORY_LIVING);
    wide.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
    wide.set_collection_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));

    let mut item = c4_definition("ITEM", "Item", "#strict 2\n");
    item.set_category(CATEGORY_OBJECT);
    item.set_collectible(true);

    let mut engine = engine_fixture([narrow, wide, item]);
    let collector = engine.spawn_object(
        SpawnConfig::new("NARR")
            .with_alive(true)
            .with_position(Vector2::new(100, 100))
            .with_loaded(true),
    )?;
    let item = engine.spawn_object(
        SpawnConfig::new("ITEM")
            .with_position(Vector2::new(105, 100))
            .with_fixed_velocity(FixedVec2::new(itofix(3), C4Fixed::ZERO))
            .with_mobile(true)
            .with_loaded(true),
    )?;

    let collector_index = engine.test_object_index(collector);
    unit_assert_ne!(
        engine.object_ocf_at_index(collector_index) & ocf::COLLECTION =>
        0,
        "the narrow definition admits the collector to CrossCheck"
    );
    let item_index = engine.test_object_index(item);
    unit_assert_eq!(
        engine.object_ocf_at_index(item_index) & (ocf::CARRYABLE | ocf::HIT_SPEED2) =>
        ocf::CARRYABLE | ocf::HIT_SPEED2,
        "the item enters both the collection and QueryCatchBlow paths"
    );

    engine.cross_check(3)?;

    let collector_index = engine.test_object_index(collector);
    unit_assert_eq!(engine.objects[collector_index].definition_id => "WIDE");
    unit_assert_eq!(
        engine.test_object_snapshot(item).container =>
        Some(collector),
        "C++ tests the live WIDE collection rectangle after QueryCatchBlow"
    );
    Ok(())
}

#[test]
fn cross_check_collection_rectangle_is_half_open_like_cpp() -> Result<(), EngineError> {
    // CrossCheck uses Inside(offset, 0, size - 1), so left/top and the last
    // pixel are included while right/bottom are excluded
    // (C4GameObjects.cpp:185-190; C4Math.h:22).
    fn collected_at(offset: Vector2) -> Result<bool, EngineError> {
        let mut collector = test_definition("COLL", "Collector", "#strict 2\n");
        collector.set_category(CATEGORY_LIVING);
        collector.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        collector.set_collection_rect(Some(DefinitionRect::new(-2, -1, 4, 3)));
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_category(CATEGORY_OBJECT);
        item.set_collectible(true);
        let mut engine = engine_fixture([collector, item]);
        let collector = spawn_fixture!(engine, "COLL", with_loaded: true, with_alive: true, with_construction: FULL_CON, with_position: Vector2::new(50, 50));
        let item = spawn_fixture!(engine, "ITEM", with_loaded: true, with_construction: FULL_CON, with_position: Vector2::new(50 + offset.x, 50 + offset.y));

        engine.cross_check(3)?;
        Ok(engine.test_object_snapshot(item).container == Some(collector))
    }

    for (offset, expected) in [
        (Vector2::new(-2, -1), true),
        (Vector2::new(1, 1), true),
        (Vector2::new(2, 0), false),
        (Vector2::new(0, 2), false),
        (Vector2::new(-3, 0), false),
        (Vector2::new(0, -2), false),
    ] {
        unit_assert_eq!(collected_at(offset)? => expected, "offset {offset:?}");
    }
    Ok(())
}

#[test]
fn command_enter_runs_collection_then_entrance_and_transfers_contents_like_cpp(
) -> Result<(), EngineError> {
    // C4Command::Enter calls cObj->Enter(Target) (C4Command.cpp:600-605).
    // C4Object::Enter links the object first, then calls Collection2 on
    // the target and Entrance on the entering object in that order
    // (C4Object.cpp:1598-1630). LORY's Entrance depends on the ordering:
    // it transfers its load with pNewContainer->GrabContents(this())
    // (Objects.c4d/Vehicles.c4d/Lorry.c4d/Script.c:82-91).
    let lorry_script = r#"#strict
local callback_order, collection_target, collection_container, entrance_target;

public func Board(pTarget)
{
  return(SetCommand(this(), "Enter", pTarget));
}

public func CollectedBy(pTarget)
{
  callback_order = callback_order * 10 + 1;
  collection_target = pTarget;
  collection_container = Contained();
  return(1);
}

protected func Entrance(pTarget)
{
  callback_order = callback_order * 10 + 2;
  entrance_target = pTarget;
  pTarget->GrabContents(this());
  return(1);
}
"#;
    let foundry_script = r#"#strict
protected func Collection2(pObject)
{
  pObject->~CollectedBy(this());
  return(1);
}
"#;
    let lorry = c4_definition("LORY", "Lorry", lorry_script);
    let mut foundry = c4_definition("FNDR", "Foundry", foundry_script);
    foundry.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    foundry.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    let mut engine = engine_fixture([lorry, foundry, simple_definition("ORE1")]);

    // A shaped spawn at y=120 grows upward to center y=100.
    let foundry_id = spawn_at_fixture(&mut engine, "FNDR", 100, 120);
    let foundry_idx = engine.test_object_index(foundry_id);
    engine.objects[foundry_idx].state.entrance_status = true;
    engine.refresh_object_ocf(foundry_idx);
    let lorry_id = spawn_at_fixture(&mut engine, "LORY", 100, 100);
    let ore_id = spawn_fixture!(engine, "ORE1", with_container: lorry_id);

    let lorry_idx = engine.test_object_index(lorry_id);
    unit_assert_eq!(engine.call_object_function(lorry_idx, "Board", vec![object_reference_value(foundry_id)],)? => Value::Bool(true));
    engine.tick_without_snapshot()?;

    let lorry_idx = engine.test_object_index(lorry_id);
    let locals = &engine.objects[lorry_idx].state.local_vars;
    unit_assert_eq!(locals.get("callback_order") => Some(&Value::Int(12)), "Collection2 must run before Entrance (C4Object.cpp:1626-1629)");
    unit_assert_eq!(locals.get("collection_target") => Some(&object_reference_value(foundry_id)));
    unit_assert_eq!(
        locals.get("collection_container") =>
        Some(&object_reference_value(foundry_id)),
        "Collection2 observes the already-linked object (C4Object.cpp:1598-1626)"
    );
    unit_assert_eq!(locals.get("entrance_target") => Some(&object_reference_value(foundry_id)));
    unit_assert_eq!(engine.objects[lorry_idx].state.container => Some(foundry_id));
    let ore_idx = engine.test_object_index(ore_id);
    unit_assert_eq!(engine.objects[ore_idx].state.container => Some(foundry_id), "LORY-style Entrance must be able to GrabContents into the new container");
    Ok(())
}

#[test]
fn automatic_collection_runs_collection_then_entrance_like_cpp() -> Result<(), EngineError> {
    // C4Object::Collect calls Enter(collector, true, false,
    // &reject_collect) (C4Object.cpp:5696-5704), so successful automatic
    // collection shares Enter's Collection2-before-Entrance callback
    // order (C4Object.cpp:1625-1630). Tutorial08 relies on WIPF::Entrance
    // when caught animals subsequently enter a LORY.
    let collector_script = r#"#strict
protected func Collection2(pObject)
{
  pObject->~CollectedBy(this());
  return(1);
}
"#;
    let item_script = r#"#strict
local callback_order, collection_target, entrance_target;

public func CollectedBy(pTarget)
{
  callback_order = callback_order * 10 + 1;
  collection_target = pTarget;
  return(1);
}

protected func Entrance(pTarget)
{
  callback_order = callback_order * 10 + 2;
  entrance_target = pTarget;
  return(1);
}
"#;
    let mut collector = c4_definition("CLNK", "Clonk", collector_script);
    collector.set_shape_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    collector.set_collection_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    let mut item = c4_definition("WIPF", "Wipf", item_script);
    item.set_collectible(true);
    let mut engine = engine_fixture([collector, item]);

    let collector_id = spawn_living_at_fixture(&mut engine, "CLNK", 100, 100);
    let item_id = spawn_category_at_fixture(&mut engine, "WIPF", CATEGORY_VEHICLE, 100, 95);

    advance_fixture(&mut engine, 3)?;

    let item_idx = engine.test_object_index(item_id);
    unit_assert_eq!(engine.objects[item_idx].state.container => Some(collector_id));
    let locals = &engine.objects[item_idx].state.local_vars;
    unit_assert_eq!(locals.get("callback_order") => Some(&Value::Int(12)), "Collect must reuse C4Object::Enter's callback order");
    unit_assert_eq!(locals.get("collection_target") => Some(&object_reference_value(collector_id)));
    unit_assert_eq!(locals.get("entrance_target") => Some(&object_reference_value(collector_id)));
    Ok(())
}

#[test]
fn automatic_collection_keeps_item_motion_through_callbacks_then_copies_collector(
) -> Result<(), EngineError> {
    // C4Object::Collect is the only Enter caller with fCopyMotion=false.
    // Collection2, Entrance, Collection and Hit therefore observe the
    // carryable's incoming motion. ObjectComCancelAttach precedes
    // Collection, and only the final Collect tail copies the collector's
    // current fixed motion (C4Object.cpp:5698-5713).
    let collector_script = r#"#strict
protected func RejectCollect(idItem, pItem)
{
  pItem->Mark(2);
  return(0);
}

protected func Collection2(pItem)
{
  pItem->Mark(3);
  return(1);
}

protected func Collection(pItem)
{
  pItem->RecordCollectionState();
  return(1);
}
"#;
    let item_script = r#"#strict
local callback_order;
local entrance_x, entrance_xdir;
local collection_x, collection_xdir;
local hit_x, hit_xdir;

public func Mark(iStep)
{
  callback_order = callback_order * 10 + iStep;
  return(1);
}

protected func RejectEntrance(pContainer)
{
  Mark(1);
  return(0);
}

protected func Entrance(pContainer)
{
  Mark(4);
  entrance_x = GetX();
  entrance_xdir = GetXDir();
  return(1);
}

public func RecordCollectionState()
{
  if (GetAction() eq "Idle") Mark(5); else Mark(9);
  collection_x = GetX();
  collection_xdir = GetXDir();
  return(1);
}

protected func Hit()
{
  Mark(6);
  hit_x = GetX();
  hit_xdir = GetXDir();
  return(1);
}
"#;
    let mut collector = c4_definition("ACOL", "Automatic collector", collector_script);
    collector.set_shape_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    collector.set_collection_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    let mut item = c4_definition("AITE", "Automatic item", item_script);
    item.set_collectible(true);
    item.configure_actions(
        None,
        HashMap::from([(
            "Attached".to_string(),
            ActionSpec::default().with_procedure("ATTACH"),
        )]),
    );

    let mut engine = engine_fixture([collector, item]);
    let collector_id = spawn_living_at_fixture(&mut engine, "ACOL", 100, 100);
    let item_id = engine.spawn_object(
        SpawnConfig::new("AITE")
            .with_action(ActionState::new("Attached"))
            .with_position(Vector2::new(104, 95)),
    )?;

    let collector_idx = engine.test_object_index(collector_id);
    engine.objects[collector_idx].fixed_velocity = FixedVec2::new(
        C4Fixed::from_raw(212_992), // 3.25
        C4Fixed::from_raw(-98_304), // -1.5
    );
    let collector_position = engine.objects[collector_idx].state.position;
    let collector_velocity = engine.objects[collector_idx].fixed_velocity;
    let item_idx = engine.test_object_index(item_id);
    engine.objects[item_idx].fixed_velocity.x = C4Fixed::from_raw(114_688); // 1.75
    engine.refresh_object_ocf(item_idx);
    let incoming_position = engine.objects[item_idx].state.position;

    engine.cross_check(3)?;

    let item_idx = engine.test_object_index(item_id);
    let item = &engine.objects[item_idx];
    unit_assert_eq!(item.state.container => Some(collector_id));
    unit_assert_eq!(item.state.action.name => "Idle");
    unit_assert_eq!(
        item.state.local_vars.get("callback_order") =>
        Some(&Value::Int(123_456)),
        "RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> \
             CancelAttach -> Collection -> Hit"
    );
    for field in ["entrance_x", "collection_x", "hit_x"] {
        unit_assert_eq!(item.state.local_vars.get(field) => Some(&Value::Int(incoming_position.x)), "{field} observes the pre-CopyMotion item position");
    }
    for field in ["entrance_xdir", "collection_xdir", "hit_xdir"] {
        unit_assert_eq!(item.state.local_vars.get(field) => Some(&Value::Int(18)), "{field} observes the incoming 1.75 fixed xdir");
    }
    unit_assert_eq!(item.state.position => collector_position);
    unit_assert_eq!(item.fixed_velocity => collector_velocity);
    unit_assert_eq!(
        item.fixed_position =>
        FixedVec2::from_ints(collector_position.x, collector_position.y),
        "the post-Hit CopyMotion snaps fixed position to collector integers"
    );
    Ok(())
}

// FnCollect is the script-facing C4Object::Collect path used by the
// shipped Alchemy MFBL spell after it creates FRBL. C++ runs both Enter
// vetoes before mutation, then Collection2 -> Entrance -> Collection ->
// Hit, and only afterwards copies the collector motion onto an item that
// stayed contained (C4Script.cpp:391-415; C4Object.cpp:1566-1636,
// 5693-5714).
#[test]
fn script_collect_preserves_cpp_callback_and_motion_order() -> Result<(), EngineError> {
    let collector_script = r#"#strict
local reject_saw_collection, reject_saw_delay;

public func Take(pItem) { return(Collect(pItem)); }

protected func RejectCollect(idItem, pItem)
{
  reject_saw_collection = !!(GetOCF() & OCF_Collection);
  reject_saw_delay = GetObjectVal("NoCollectDelay");
  pItem->Mark(2);
  return(0);
}

protected func Collection2(pItem)
{
  pItem->Mark(3);
  return(1);
}

protected func Collection(pItem)
{
  pItem->Mark(5);
  return(1);
}
"#;
    let item_script = r#"#strict
local callback_order, hit_x, hit_xdir;

public func Mark(iStep)
{
  callback_order = callback_order * 10 + iStep;
  return(1);
}

protected func RejectEntrance(pContainer)
{
  Mark(1);
  return(0);
}

protected func Entrance(pContainer)
{
  Mark(4);
  return(1);
}

protected func Hit()
{
  Mark(6);
  hit_x = GetX();
  hit_xdir = GetXDir();
  return(1);
}
"#;
    let mut collector = c4_definition("COLL", "Collector", collector_script);
    collector.set_collection_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    collector.set_collection_limit(2);
    let mut item = c4_definition("ITEM", "Item", item_script);
    item.set_collectible(true);
    item.configure_actions(
        None,
        HashMap::from([(
            "Attached".to_string(),
            ActionSpec::default().with_procedure("ATTACH"),
        )]),
    );
    let mut engine = engine_fixture([collector, item]);

    let collector_id = engine.spawn_object(
        SpawnConfig::new("COLL")
            .with_owner(1)
            .with_controller(7)
            .with_position(Vector2::new(100, 80))
            .with_velocity(Vector2::new(3, -2)),
    )?;
    let item_id = engine.spawn_object(
        SpawnConfig::new("ITEM")
            .with_owner(2)
            .with_controller(2)
            .with_action(ActionState::new("Attached"))
            .with_position(Vector2::new(40, 50))
            .with_velocity(Vector2::new(1, 0))
            .with_mobile(false),
    )?;
    let item_idx = engine.test_object_index(item_id);
    engine.objects[item_idx].fixed_velocity.x = C4Fixed::from_raw(114_688); // 1.75
    engine.refresh_object_ocf(item_idx);

    let collector_idx = engine.test_object_index(collector_id);
    engine.objects[collector_idx].state.no_collect_delay = 2;
    engine.refresh_object_ocf(collector_idx);
    unit_assert_eq!(engine.objects[collector_idx].state.ocf & ocf::COLLECTION => 0, "the armed NoCollectDelay starts with Collection suppressed");
    unit_assert_eq!(engine.call_object_function(collector_idx, "Take", vec![object_reference_value(item_id)],)? => Value::Bool(true));

    let collector_idx = engine.test_object_index(collector_id);
    let collector_position = engine.objects[collector_idx].state.position;
    let collector_velocity = engine.objects[collector_idx].fixed_velocity;
    unit_assert_eq!(
        engine.objects[collector_idx]
            .state
            .local_vars
            .get("reject_saw_collection") =>
        Some(&Value::Bool(true)),
        "FnCollect temporarily clears NoCollectDelay and updates OCF before callbacks"
    );
    unit_assert_eq!(
        engine.objects[collector_idx]
            .state
            .local_vars
            .get("reject_saw_delay") =>
        Some(&Value::Int(0)),
        "callbacks observe FnCollect's temporary zero delay"
    );
    unit_assert_eq!(engine.objects[collector_idx].state.no_collect_delay => 2, "FnCollect restores the larger pre-call delay");
    unit_assert_ne!(engine.objects[collector_idx].state.ocf & ocf::COLLECTION => 0, "restoring NoCollectDelay does not run a second UpdateOCF in C++");
    let item_idx = engine.test_object_index(item_id);
    let item = &engine.objects[item_idx];
    unit_assert_eq!(item.state.container => Some(collector_id));
    unit_assert_eq!(item.state.controller => 7, "a nonliving collected object inherits the collector controller");
    unit_assert_eq!(item.state.action.name => "Idle", "ObjectComCancelAttach cancels ATTACH after Enter and before Collection");
    unit_assert_eq!(
        item.state.local_vars.get("callback_order") =>
        Some(&Value::Int(123_456)),
        "RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> Collection -> Hit"
    );
    unit_assert_eq!(item.state.local_vars.get("hit_x") => Some(&Value::Int(40)), "Hit observes pre-CopyMotion position");
    unit_assert_eq!(item.state.local_vars.get("hit_xdir") => Some(&Value::Int(18)), "Hit observes pre-CopyMotion fixed velocity");
    unit_assert_eq!(item.state.position => collector_position);
    unit_assert_eq!(item.fixed_velocity => collector_velocity);
    unit_assert!(!item.state.mobile, "Collect's delayed CopyMotion preserves the item's Mobile flag");

    let second_item = engine.spawn_object(
        SpawnConfig::new("ITEM")
            .with_owner(2)
            .with_position(Vector2::new(60, 50)),
    )?;
    let collector_idx = engine.test_object_index(collector_id);
    unit_assert_eq!(
        engine.call_object_function(
            collector_idx,
            "Take",
            vec![object_reference_value(second_item)],
        )? =>
        Value::Bool(true),
        "the second free slot remains collectable despite NoCollectDelay"
    );
    let collector_idx = engine.test_object_index(collector_id);
    unit_assert_eq!(engine.objects[collector_idx].state.ocf & ocf::COLLECTION => 0, "Enter clears OCF_Collection as soon as CollectionLimit is reached");
    Ok(())
}

// CrossCheck collection routes through C4Object::Collect -> Enter
// (C4GameObjects.cpp:190, C4Object.cpp:5698 -> :1552): the FIRST gate is
// the collected object's RejectEntrance callback (C4Object.cpp:1564) —
// a truthy return aborts BEFORE any state change. The GoldRush wipf 564
// "collecting" walking wipf 563 (HitSpeed2 carryable) is vetoed exactly
// here (ANIM's RejectEntrance) — C++ leaves 563's position/velocity
// untouched; teleporting it to the collector was the f41 wall.
#[test]
fn cross_check_collection_reject_entrance_veto_leaves_object_untouched_like_cpp(
) -> Result<(), EngineError> {
    let mut collector = test_definition("Croc", "Croc", "#strict\n");
    collector.set_shape_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    collector.set_collection_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    let item_script = r#"#strict
protected func RejectEntrance(pContainer) { return(1); }
"#;
    let mut item = c4_definition("Chip", "Chip", item_script);
    item.set_collectible(true);
    let mut engine = engine_fixture([collector, item]);

    let collector_id = spawn_living_at_fixture(&mut engine, "Croc", 100, 100);
    // A fast carryable INSIDE the collector's shape: OCF_HitSpeed2 makes
    // it a reverse-pass candidate on EVERY frame (tocf |= OCF_HitSpeed2,
    // C4GameObjects.cpp:148), and the collection arm checks the RAW OCFs
    // (:186) — no Tick3 gate applies to this pairing.
    // Spawn y bottom-anchors the SHAPED collector (C4Object.cpp:1462-
    // 1468): its center lands at (100, 91); the shapeless item keeps its
    // spawn point as center — y=95 puts it inside the shape (dy=4).
    let item_id = spawn_category_at_fixture(&mut engine, "Chip", CATEGORY_VEHICLE, 100, 95);
    let item_idx = engine.test_object_index(item_id);
    engine.objects[item_idx].fixed_velocity.x = C4Fixed::from_raw(147456); // 2.25
    engine.objects[item_idx].state.mobile = true;

    engine.tick_without_snapshot()?;

    let item_idx = engine.test_object_index(item_id);
    let item = &engine.objects[item_idx];
    unit_assert_eq!(item.state.container => None, "entrance vetoed");
    unit_assert_eq!(
        item.state.position =>
        Vector2::new(102, 95),
        "vetoed collection must not teleport the object (C4Object.cpp:1564 \
             returns before any state change) — it keeps its own movement"
    );
    unit_assert_eq!(item.fixed_velocity.x.val() => 147456, "vetoed collection must not zero the velocity");
    let collector_idx = engine.test_object_index(collector_id);
    unit_assert!(engine.objects[collector_idx].state.contents.is_empty(), "nothing entered the collector");
    Ok(())
}

// The second Enter gate: RejectCollect on the COLLECTOR with
// (idObject, pObject) — truthy aborts before any state change
// (C4Object.cpp:1569-1577; PSF_RejectCollection = "~RejectCollect",
// C4Script.h:82). The GoldRush wipf's script ends its RejectCollect
// with return(1) — a wipf never auto-collects.
#[test]
fn cross_check_collection_reject_collect_veto_leaves_object_untouched_like_cpp(
) -> Result<(), EngineError> {
    let collector_script = r#"#strict
protected func RejectCollect(object_id, pObject) { return(1); }
"#;
    let mut collector = c4_definition("Croc", "Croc", collector_script);
    collector.set_shape_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    collector.set_collection_rect(Some(DefinitionRect::new(-12, -9, 24, 18)));
    let mut item = test_definition("Chip", "Chip", "#strict\n");
    item.set_collectible(true);
    let mut engine = engine_fixture([collector, item]);

    let collector_id = spawn_living_at_fixture(&mut engine, "Croc", 100, 100);
    let item_id = spawn_category_at_fixture(&mut engine, "Chip", CATEGORY_VEHICLE, 100, 95);
    let item_idx = engine.test_object_index(item_id);
    engine.objects[item_idx].fixed_velocity.x = C4Fixed::from_raw(147456); // 2.25
    engine.objects[item_idx].state.mobile = true;

    engine.tick_without_snapshot()?;

    let item_idx = engine.test_object_index(item_id);
    let item = &engine.objects[item_idx];
    unit_assert_eq!(item.state.container => None, "collection vetoed");
    unit_assert_eq!(item.state.position => Vector2::new(102, 95));
    unit_assert_eq!(item.fixed_velocity.x.val() => 147456);
    let collector_idx = engine.test_object_index(collector_id);
    unit_assert!(engine.objects[collector_idx].state.contents.is_empty());
    Ok(())
}

#[test]
fn auto_collect_respects_collection_limit() -> Result<(), EngineError> {
    // Bottom-anchor the crew at center y=0 (C4Object.cpp:1462-1468).
    let (mut engine, crew, [first, second]) = auto_collect_fixture(Some(1), [3, -3]);

    // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
    advance_fixture(&mut engine, 3)?;

    let first_snapshot = engine.test_object_snapshot(first);
    let second_snapshot = engine.test_object_snapshot(second);
    let collected = [first_snapshot.container, second_snapshot.container];
    unit_assert_eq!(collected.iter().filter(|entry| entry.is_some()).count() => 1, "exactly one item should be collected due to the limit");
    unit_assert_eq!(first_snapshot.container => Some(crew));
    unit_assert!(second_snapshot.container.is_none());
    Ok(())
}

#[test]
fn try_enter_nearby_moves_crew_into_structure() -> Result<(), EngineError> {
    let mut crew_definition = test_definition("Crew", "Crew", BASIC_OBJECT_SCRIPT);
    crew_definition.set_crew_member(true);
    crew_definition.set_movement_profile(MovementProfile::default());
    let mut structure_definition = test_definition("Hut", "Hut", BASIC_OBJECT_SCRIPT);
    structure_definition.set_ocf_base(ocf::ENTRANCE | ocf::CONTAINER);
    let mut engine = engine_fixture([crew_definition, structure_definition]);

    // Bottom-anchor the crew at center y=0 (C4Object.cpp:1462-1468).
    let crew = spawn_crew_at_fixture(&mut engine, "Crew", 1, 0, 16);
    engine.select_crew(1, vec![crew])?;
    engine.set_crew_cursor(1, Some(crew))?;
    let hut = spawn_at_fixture(&mut engine, "Hut", 0, 0);

    unit_assert!(engine.try_enter_nearby(1)?);
    let crew_snapshot = engine.test_object_snapshot(crew);
    unit_assert_eq!(crew_snapshot.container => Some(hut));
    Ok(())
}

#[test]
fn acquire_preserves_walk_trajectory_across_frames() {
    let script = "#strict 2";
    let mut walker = test_definition("L111", "L111 walker", script);
    walker.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("WALK"),
        )]),
    );
    // Keep Physical.Walk at zero so the deterministic fixture uses the
    // compact MovementProfile path.
    walker.set_movement_profile(
        MovementProfile::default()
            .with_walk_speed(8)
            .with_walk_acceleration(2),
    );

    let mut engine = Engine::with_seed(111);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_definition(walker);

    let spawn = |y| {
        SpawnConfig::new("L111")
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::new(100, y))
            .with_action(ActionState::new("Walk"))
            .with_command_direction(CommandDirection::Right)
            .with_alive(true)
            .with_mobile(true)
    };
    let control = engine.spawn_test_object(spawn(20));
    let acquiring = engine.spawn_test_object(spawn(80));

    let acquiring_index = engine.test_object_index(acquiring);
    engine.objects[acquiring_index]
        .commands
        .push_back(
            CommandRequest::new(CommandId::Acquire)
                .with_data(CommandData::Text("WOOD".into()))
                .with_mode(CommandMode::Base),
        )
        .test_value();

    let start_x = 100;
    let mut final_x = start_x;
    for frame in 1..=6 {
        let snapshot = engine.test_tick();
        let control_state = snapshot.object(control).test_value();
        let acquiring_state = snapshot.object(acquiring).test_value();

        unit_assert_eq!(control_state.command_direction => CommandDirection::Right, "frame {frame}");
        unit_assert_eq!(acquiring_state.command_direction => CommandDirection::Right, "Acquire must not overwrite ComDir on frame {frame}");
        unit_assert_eq!(acquiring_state.position.x => control_state.position.x, "frame {frame}");
        unit_assert_eq!(acquiring_state.velocity => control_state.velocity, "frame {frame}");
        unit_assert_eq!(acquiring_state.fixed_velocity => control_state.fixed_velocity, "subpixel trajectory differs on frame {frame}");
        unit_assert_eq!(acquiring_state.command_stack.command_names() => vec!["Acquire".to_string()], "handled Acquire must remain active on frame {frame}");
        final_x = acquiring_state.position.x;

        // Drive the asynchronous script response directly so this replay
        // isolates Acquire's command delta from optional-callback lookup
        // and the separate InitEvaluation timing slice.
        let acquiring_index = engine.test_object_index(acquiring);
        unit_assert!(engine.objects[acquiring_index].commands.set_acquire_script_result(AcquireScriptResult::Handled));
    }
    unit_assert!(final_x > start_x, "the preserved Right ComDir must move the actor");
}

#[test]
fn get_plr_control_name_uses_runtime_binding_names_and_empty_string_failures() {
    let mut engine = Engine::new();
    let mut set_two = vec![ControlKeyName::new("", ""); 12];
    set_two[6] = ControlKeyName::new("Left Arrow", "Left");
    engine.set_control_key_names(HashMap::from([(2, set_two)]));
    engine
        .register_player_with_runtime_control(
            PlayerConfig::new(9, "Ada"),
            PlayerRuntimeControl::new(2, 0),
        )
        .test_value();
    engine.register_test_definition(test_definition(
        "KYNM",
        "Control name probe",
        r#"#strict 2
                func Probe() {
        return [GetPlrControlName(9, 6), GetPlrControlName(9, 6, true),
                GetPlrControlName(42, 6), GetPlrControlName(9, 99)];
                }
                "#,
    ));
    let object = engine.spawn_test_object(SpawnConfig::new("KYNM"));
    let index = engine.test_object_index(object);

    unit_assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("GetPlrControlName calls run") =>
        Value::Array(vec![
            Value::String("Left Arrow".into()),
            Value::String("Left".into()),
            Value::String(String::new().into()),
            Value::String(String::new().into()),
        ])
    );
}

#[test]
fn nil_map_assignment_matches_cpp_removal_and_unchanged_slot_rules() {
    // Pre-STRICT3 code can receive maps but cannot spell map literals or
    // dot access. Host-provided maps plus bracket access keep both the
    // STRICT2 and STRICT3 assignment paths covered with C++-valid syntax.
    let script_template = r#"#strict {strict}
func Probe(removed, fresh, reduced, already_nil,
           concat_removed, concat_fresh, remove_patch, fresh_patch) {
    var unset;
    removed["a"] = unset;
    fresh["a"] = unset;
    reduced["b"] = unset;
    already_nil["a"] = unset;
    concat_removed = concat_removed .. remove_patch;
    concat_fresh = concat_fresh .. fresh_patch;
    return [GetLength(removed), GetLength(fresh), GetLength(reduced),
            GetLength(already_nil), GetLength(concat_removed),
            GetLength(concat_fresh), removed, fresh, reduced, already_nil,
            concat_removed, concat_fresh];
}
"#;
    for (id, strict) in [("MN02", 2), ("MN03", 3)] {
        let script = script_template.replace("{strict}", &strict.to_string());
        let mut engine = Engine::new();
        engine.register_test_script_definition(id, "Map nil removal", &script);
        let object = engine.spawn_test_object(SpawnConfig::new(id));
        let index = engine.test_object_index(object);
        let result = engine.call_test_object_function(
            index,
            "Probe",
            vec![
                Value::Proplist(clonk_script::ValueMap::from([("a", Value::Int(1))])),
                Value::Proplist(clonk_script::ValueMap::new()),
                Value::Proplist(clonk_script::ValueMap::from([
                    ("a", Value::Int(1)),
                    ("b", Value::Int(2)),
                ])),
                Value::Proplist(clonk_script::ValueMap::from([("a", Value::Nil)])),
                Value::Proplist(clonk_script::ValueMap::from([("a", Value::Int(1))])),
                Value::Proplist(clonk_script::ValueMap::new()),
                Value::Proplist(clonk_script::ValueMap::from([("a", Value::Nil)])),
                Value::Proplist(clonk_script::ValueMap::from([("a", Value::Nil)])),
            ],
        );
        let Value::Array(values) = result else {
            panic!("map removal probe must return an array");
        };

        unit_assert_eq!(values[..6] => [Value::Int(0), Value::Int(1), Value::Int(1), Value::Int(1), Value::Int(0), Value::Int(1)]);
        let empty = Value::Proplist(clonk_script::ValueMap::new());
        let reduced = Value::Proplist(clonk_script::ValueMap::from([("a", Value::Int(1))]));
        let nil_entry = Value::Proplist(clonk_script::ValueMap::from([("a", Value::Nil)]));
        unit_assert_eq!(values[6] => empty);
        unit_assert_eq!(values[7] => nil_entry);
        unit_assert_eq!(values[8] => reduced);
        unit_assert_eq!(values[9] => nil_entry);
        unit_assert_eq!(values[10] => empty);
        unit_assert_eq!(values[11] => nil_entry);
        unit_assert_eq!(values[6].c4_value_hash() => empty.c4_value_hash());
        unit_assert_eq!(values[7].c4_value_hash() => nil_entry.c4_value_hash());
        unit_assert_eq!(values[8].c4_value_hash() => reduced.c4_value_hash());
    }
}
