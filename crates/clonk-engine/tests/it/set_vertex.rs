use std::fs;
use std::path::Path;

use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_resources::{Group, ResourceDefinition};
use clonk_script::Value;
use tempfile::tempdir;

fn write_definition(path: &Path, def_core: &str, script: &str) {
    fs::create_dir_all(path).expect("definition directory creates");
    fs::write(path.join("DefCore.txt"), def_core).expect("DefCore writes");
    fs::write(path.join("Script.c"), script).expect("Script writes");
}

fn probe(def_core: &str, script: &str) -> Value {
    let root = tempdir().expect("resource root creates");
    let path = root.path().join("Probe.c4d");
    write_definition(&path, def_core, script);
    let resource = ResourceDefinition::load(&Group::open(&path).expect("group opens"))
        .expect("definition loads");
    let mut engine = Engine::new();
    engine
        .register_definition(Definition::from_resource(&resource).expect("definition compiles"))
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("PRB1").with_loaded(true))
        .expect("object spawns");
    let index = engine.find_object_index(object).expect("object exists");
    engine
        .call_object_function(index, "Probe", vec![])
        .expect("probe runs")
}

/// `SetVertex(..., VTX_SetPermanentUpd)` writes the own-vertex backup at
/// `iIndex + C4D_VertexCpyPos` and then runs `UpdateShape(true)`, which copies
/// `VtxNum = min(VtxNum, C4D_VertexCpyPos)` vertices back from that backup into
/// the live shape (oracle-src-pinned src/C4Script.cpp:1292-1326,
/// src/C4Object.cpp:322-350, src/C4Shape.cpp:421-437, :484-494). The live
/// vertex count therefore stays at the definition's, and `GetVertex` — which
/// reads the live slots (src/C4Script.cpp:1270-1289) — observes the new value.
#[test]
fn set_vertex_permanent_update_mode_rebuilds_the_live_shape_from_the_own_copy() {
    let result = probe(
        "[DefCore]\nid=PRB1\nName=PRB1\nWidth=25\nHeight=25\nOffset=-12,-12\n\
         Vertices=1\nVertexY=1\nVertexCNAT=64\nRotate=1\n",
        r#"#strict 2
func Probe()
{
  SetVertex(0, 1, 60, 0, 2);
  return [GetVertex(0, 0), GetVertex(0, 1), GetVertexNum()];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![Value::Int(0), Value::Int(60), Value::Int(1)]),
        "Hazard's crosshair sets its attach vertex 60 pixels out with \
         SetVertex(0,1,CH_Distance,0,2); the live shape must carry that vertex"
    );
}

/// The permanent-update copy runs through `UpdateShape`, so `Shape.Rotate`
/// applies the object's rotation to the restored vertices for a `Rotate=1`
/// definition (src/C4Object.cpp:343-346, src/C4Shape.cpp:41-70). Hazard's
/// crosshair rides on exactly this: `SetR` swings its 60-pixel attach vertex
/// around the aiming Clonk (Hazard.c4d/Crew.c4d/HazardClonk.c4d/
/// Crosshair.c4d/Script.c:16-19,45-48).
#[test]
fn set_vertex_permanent_update_mode_reapplies_the_object_rotation() {
    let result = probe(
        "[DefCore]\nid=PRB1\nName=PRB1\nWidth=25\nHeight=25\nOffset=-12,-12\n\
         Vertices=1\nVertexY=1\nVertexCNAT=64\nRotate=1\n",
        r#"#strict 2
func Probe()
{
  SetVertex(0, 1, 60, 0, 2);
  SetR(90);
  return [GetVertex(0, 0), GetVertex(0, 1)];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![Value::Int(-60), Value::Int(0)]),
        "C4Shape::Rotate maps (0,60) at r=90 onto (-60,0)"
    );
}

/// Plain `SetVertex` (no own-vertex mode) writes the live `C4Shape` slot and
/// leaves `fOwnVertices` alone (src/C4Script.cpp:1296-1323). Western's
/// `SetVertexXY` global rides on exactly this
/// (planet/System.c4g/Commits.c:68-76) to swing the Winchester crosshair.
#[test]
fn plain_set_vertex_writes_the_live_shape_slot() {
    let result = probe(
        "[DefCore]\nid=PRB1\nName=PRB1\nWidth=7\nHeight=7\nOffset=-3,-3\nVertices=1\n",
        r#"#strict 2
func Probe()
{
  SetVertex(0, 0, -40);
  SetVertex(0, 1, 4);
  return [GetVertex(0, 0), GetVertex(0, 1), GetVertexNum()];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![Value::Int(-40), Value::Int(4), Value::Int(1)])
    );
}

/// `FnSetVertex`'s `pObj` argument targets another object entirely
/// (src/C4Script.cpp:1294-1295). Western's `SetVertexXY` global forwards it,
/// so the Cowboy's watch idle re-aims the Winchester crosshair from its own
/// script context (Western.c4d/Crew.c4d/Cowboy.c4d/Script.c:700).
#[test]
fn plain_set_vertex_reaches_a_foreign_target() {
    let result = probe(
        "[DefCore]\nid=PRB1\nName=PRB1\nWidth=7\nHeight=7\nOffset=-3,-3\nVertices=1\n",
        r#"#strict 2
func Probe()
{
  var obj = CreateObject(PRB1, 0, 0, -1);
  SetVertex(0, 0, -40, obj);
  SetVertex(0, 1, 4, obj);
  return [GetVertex(0, 0, obj), GetVertex(0, 1, obj), GetVertexNum(obj)];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![Value::Int(-40), Value::Int(4), Value::Int(1)])
    );
}
