use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_resources::MaterialLibrary;
use clonk_script::Value;

fn string(value: &str) -> Value {
    Value::String(value.to_owned().into())
}

#[test]
fn get_material_val_reflects_compiled_material_core() {
    let library = crate::support::TestValueExt::test_value(MaterialLibrary::parse(
        r#"
        [Material CoreProbe]
        Name=CoreProbe
        Color=1,2,3,4
        ColorX=9,0,8
        Alpha=7,0,6
        Density=80
        DigFree=1
        BlastFree=0
        Blast2Object=ROCKsuffix
        Dig2Object=ROCK
        Dig2ObjectRequest=1
        TextureOverlay=Smooth extra
        PXSGfxRt=11,12,13,14,15,16

        [Reaction]
        Type="Poof"
        TargetSpec="Solid"

        [Reaction]
        Type=Convert
        TargetSpec=Liquid
        ExecMask=4
        Reverse=1
        InverseSpec=1
        CheckSlide=0
        Depth=-3
        ConvertMat="Water"
        CorrosionRate=17

        [Material NegativePlace]
        Name=NegativePlace
        Density=80
        Placement=-7
        "#,
    ));

    let mut engine = Engine::new();
    engine.configure_materials_from_library(&library);
    crate::support::TestValueExt::test_value(engine.register_definition(
        crate::support::TestValueExt::test_value(Definition::from_script(
            "GMVL",
            "GetMaterialVal compiled-core probe",
            r#"#strict 2
                public func Probe()
                {
                var core = Material("CoreProbe");
                var negative = Material("NegativePlace");
                return [
                    // Section, entry, material, and entry-number matching is exact.
                    [GetMaterialVal("Density", "Material", core),
                     GetMaterialVal("density", "Material", core),
                     GetMaterialVal("Density", "material", core),
                     GetMaterialVal("Missing", "Material", core),
                     GetMaterialVal("Density", "Material", -1),
                     GetMaterialVal("Density", "Material", 99),
                     GetMaterialVal("Density", "Material", core, -1),
                     GetMaterialVal("Density", "Material", core, 1)],

                    // Defaults retain the native compiled type. Placement=0 is replaced
                    // by C4MaterialCore::Load's density/flags calculation, while an
                    // explicitly negative Placement remains a nonzero live value.
                    [GetMaterialVal("Name", "Material", core),
                     GetMaterialVal("Shape", "Material", core),
                     GetMaterialVal("DigFree", "Material", core),
                     GetMaterialVal("BlastFree", "Material", core),
                     GetMaterialVal("TextureOverlay", "Material", core),
                     GetMaterialVal("TextureOverlay", "Material", negative),
                     GetMaterialVal("SplashRate", "Material", core),
                     GetMaterialVal("Placement", "Material", core),
                     GetMaterialVal("Placement", "Material", negative)],

                    // Color and ColorX decompile the same final nine-element array. A
                    // short ColorX replaces Color and zero-fills its missing elements;
                    // StdArrayDefaultAdapt then omits those trailing zero defaults.
                    [GetMaterialVal("Color", "Material", core, 0),
                     GetMaterialVal("Color", "Material", core, 1),
                     GetMaterialVal("Color", "Material", core, 2),
                     GetMaterialVal("Color", "Material", core, 3),
                     GetMaterialVal("Color", "Material", core, 4),
                     GetMaterialVal("Color", "Material", core, 5),
                     GetMaterialVal("Color", "Material", core, 6),
                     GetMaterialVal("Color", "Material", core, 7),
                     GetMaterialVal("Color", "Material", core, 8),
                     GetMaterialVal("Color", "Material", core, 9),
                     GetMaterialVal("ColorX", "Material", core, 0),
                     GetMaterialVal("ColorX", "Material", core, 1),
                     GetMaterialVal("ColorX", "Material", core, 2),
                     GetMaterialVal("ColorX", "Material", core, 3),
                     GetMaterialVal("ColorX", "Material", core, 4),
                     GetMaterialVal("ColorX", "Material", core, 5),
                     GetMaterialVal("ColorX", "Material", core, 6),
                     GetMaterialVal("ColorX", "Material", core, 7),
                     GetMaterialVal("ColorX", "Material", core, 8),
                     GetMaterialVal("ColorX", "Material", core, 9)],

                    [GetMaterialVal("Alpha", "Material", core, 0),
                     GetMaterialVal("Alpha", "Material", core, 1),
                     GetMaterialVal("Alpha", "Material", core, 2),
                     GetMaterialVal("Alpha", "Material", core, 3),
                     GetMaterialVal("Alpha", "Material", core, 4),
                     GetMaterialVal("Alpha", "Material", core, 5),
                     GetMaterialVal("Alpha", "Material", core, 6)],

                    // PXSGfxSize defaults to the compiled rectangle width.
                    [GetMaterialVal("PXSGfxRt", "Material", core, 0),
                     GetMaterialVal("PXSGfxRt", "Material", core, 1),
                     GetMaterialVal("PXSGfxRt", "Material", core, 2),
                     GetMaterialVal("PXSGfxRt", "Material", core, 3),
                     GetMaterialVal("PXSGfxRt", "Material", core, 4),
                     GetMaterialVal("PXSGfxRt", "Material", core, 5),
                     GetMaterialVal("PXSGfxRt", "Material", core, 6),
                     GetMaterialVal("PXSGfxSize", "Material", core, 0),
                     GetMaterialVal("PXSGfxSize", "Material", core, 1)],

                    // C4ID fields remain IDs; a compiled zero C4ID is canonical nil.
                    [GetMaterialVal("Dig2Object", "Material", core),
                     GetMaterialVal("Dig2Object", "Material", core, 1),
                     GetMaterialVal("Blast2Object", "Material", core),
                     GetMaterialVal("Blast2Object", "Material", negative)],

                    // Reaction is nested below Material. Its leaf compile names still
                    // index across every reaction, including native defaults and bools.
                    [GetMaterialVal("Type", "Material", core, 0),
                     GetMaterialVal("Type", "Material", core, 1),
                     GetMaterialVal("Type", "Material", core, 2),
                     GetMaterialVal("type", "Material", core, 0),
                     GetMaterialVal("TargetSpec", "Material", core, 0),
                     GetMaterialVal("TargetSpec", "Material", core, 1),
                     GetMaterialVal("ScriptFunc", "Material", core, 0),
                     GetMaterialVal("ExecMask", "Material", core, 0),
                     GetMaterialVal("ExecMask", "Material", core, 1),
                     GetMaterialVal("Reverse", "Material", core, 0),
                     GetMaterialVal("Reverse", "Material", core, 1),
                     GetMaterialVal("InverseSpec", "Material", core, 0),
                     GetMaterialVal("InverseSpec", "Material", core, 1),
                     GetMaterialVal("CheckSlide", "Material", core, 0),
                     GetMaterialVal("CheckSlide", "Material", core, 1),
                     GetMaterialVal("Depth", "Material", core, 0),
                     GetMaterialVal("Depth", "Material", core, 1),
                     GetMaterialVal("ConvertMat", "Material", core, 0),
                     GetMaterialVal("ConvertMat", "Material", core, 1),
                     GetMaterialVal("CorrosionRate", "Material", core, 0),
                     GetMaterialVal("CorrosionRate", "Material", core, 1)]
                ];
                }
                "#,
        )),
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("GMVL")));
    let probe_index = crate::support::TestValueExt::test_value(engine.find_object_index(probe));

    assert_eq!(
        engine
            .call_object_function(probe_index, "Probe", Vec::new())
            .expect("compiled GetMaterialVal reflection executes"),
        Value::Array(vec![
            Value::Array(vec![
                Value::Int(80),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ]),
            Value::Array(vec![
                string("CoreProbe"),
                Value::Int(0),
                Value::Int(1),
                Value::Int(0),
                string("Smooth"),
                string(""),
                Value::Int(10),
                Value::Int(40),
                Value::Int(-7),
            ]),
            Value::Array(vec![
                Value::Int(9),
                Value::Int(0),
                Value::Int(8),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(9),
                Value::Int(0),
                Value::Int(8),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ]),
            Value::Array(vec![
                Value::Int(7),
                Value::Int(0),
                Value::Int(6),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ]),
            Value::Array(vec![
                Value::Int(11),
                Value::Int(12),
                Value::Int(13),
                Value::Int(14),
                Value::Int(15),
                Value::Int(16),
                Value::Nil,
                Value::Int(13),
                Value::Nil,
            ]),
            Value::Array(vec![
                Value::C4Id("ROCK".to_owned()),
                Value::Nil,
                Value::C4Id("ROCK".to_owned()),
                Value::Nil,
            ]),
            Value::Array(vec![
                string("Poof"),
                string("Convert"),
                Value::Nil,
                Value::Nil,
                string("Solid"),
                string("Liquid"),
                string(""),
                Value::Int(-1),
                Value::Int(4),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(false),
                Value::Int(0),
                Value::Int(-3),
                string(""),
                string("Water"),
                Value::Int(100),
                Value::Int(17),
            ]),
        ])
    );
}
