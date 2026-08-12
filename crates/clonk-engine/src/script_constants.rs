//! The C4Script engine constants (C4ScriptConstMap, C4Script.cpp:6208-6546,
//! registered via RegisterGlobalConstant at :6581): 292 int constants every
//! script engine knows. Values resolved from the C++ headers; notable ones —
//! C4D_All = ~C4D_None over int32 (C4Def.h:42), OCF_Alive = 1<<31
//! (C4Constants.h:137), SkyPar_Keep = -163764 (C4Script.cpp:4948),
//! C4D_GrabGet/GrabPut intentionally swapped vs name order (C4Def.h:80-81).

use clonk_script::{Engine as ScriptEngine, Value};

#[rustfmt::skip]
pub(crate) const C4_SCRIPT_CONSTANTS: &[(&str, i32)] = &[
    ("C4D_All", 4294967295u32 as i32), ("C4D_StaticBack", 1), ("C4D_Structure", 2), ("C4D_Vehicle", 4), ("C4D_Living", 8), ("C4D_Object", 16), ("C4D_Goal", 32),
    ("C4D_Environment", 64), ("C4D_Knowledge", 1024), ("C4D_Magic", 131072), ("C4D_Rule", 524288), ("C4D_Background", 1048576), ("C4D_Parallax", 2097152),
    ("C4D_MouseSelect", 4194304), ("C4D_Foreground", 8388608), ("C4D_MouseIgnore", 16777216), ("C4D_IgnoreFoW", 33554432), ("C4D_GrabGet", 2), ("C4D_GrabPut", 1),
    ("C4D_LinePower", 1), ("C4D_LineSource", 2), ("C4D_LineDrain", 3), ("C4D_LineLightning", 4), ("C4D_LineVolcano", 5), ("C4D_LineRope", 6), ("C4D_LineColored", 7),
    ("C4D_LineVertex", 8), ("C4D_PowerInput", 1), ("C4D_PowerOutput", 2), ("C4D_LiquidInput", 4), ("C4D_LiquidOutput", 8), ("C4D_PowerGenerator", 16), ("C4D_PowerConsumer", 32),
    ("C4D_LiquidPump", 64), ("C4D_EnergyHolder", 256), ("C4V_Any", 0), ("C4V_Int", 1), ("C4V_Bool", 2), ("C4V_C4ID", 3), ("C4V_C4Object", 4), ("C4V_String", 5), ("C4V_Array", 6),
    ("C4V_Map", 7), ("COMD_None", 0), ("COMD_Stop", 0), ("COMD_Up", 1), ("COMD_UpRight", 2), ("COMD_Right", 3), ("COMD_DownRight", 4), ("COMD_Down", 5), ("COMD_DownLeft", 6),
    ("COMD_Left", 7), ("COMD_UpLeft", 8), ("DIR_Left", 0), ("DIR_Right", 1), ("CON_CursorLeft", 0), ("CON_CursorToggle", 1), ("CON_CursorRight", 2), ("CON_Throw", 3),
    ("CON_Up", 4), ("CON_Dig", 5), ("CON_Left", 6), ("CON_Down", 7), ("CON_Right", 8), ("CON_Menu", 9), ("CON_Special", 10), ("CON_Special2", 11), ("OCF_Construct", 2),
    ("OCF_Grab", 4), ("OCF_Collectible", 8), ("OCF_OnFire", 16), ("OCF_HitSpeed1", 32), ("OCF_Fullcon", 64), ("OCF_Inflammable", 128), ("OCF_Chop", 256), ("OCF_Rotate", 512),
    ("OCF_Exclusive", 1024), ("OCF_Entrance", 2048), ("OCF_HitSpeed2", 4096), ("OCF_HitSpeed3", 8192), ("OCF_Collection", 16384), ("OCF_Living", 32768), ("OCF_HitSpeed4", 65536),
    ("OCF_FightReady", 131072), ("OCF_LineConstruct", 262144), ("OCF_Prey", 524288), ("OCF_AttractLightning", 1048576), ("OCF_NotContained", 2097152), ("OCF_CrewMember", 4194304),
    ("OCF_Edible", 8388608), ("OCF_InLiquid", 16777216), ("OCF_InSolid", 33554432), ("OCF_InFree", 67108864), ("OCF_Available", 134217728), ("OCF_PowerConsumer", 268435456),
    ("OCF_PowerSupply", 536870912), ("OCF_Container", 1073741824), ("OCF_Alive", 2147483648u32 as i32), ("VIS_All", 0), ("VIS_None", 1), ("VIS_Owner", 2), ("VIS_Allies", 4),
    ("VIS_Enemies", 8), ("VIS_Local", 16), ("VIS_God", 32), ("VIS_LayerToggle", 64), ("VIS_OverlayOnly", 128), ("C4X_Ver1", 4), ("C4X_Ver2", 9), ("C4X_Ver3", 11), ("C4X_Ver4", 0),
    ("C4X_VerBuild", 362), ("SkyPar_Keep", -163764), ("C4MN_Style_Normal", 0), ("C4MN_Style_Context", 1), ("C4MN_Style_Info", 2), ("C4MN_Style_Dialog", 3),
    ("C4MN_Style_EqualItemHeight", 128), ("C4MN_Extra_None", 0), ("C4MN_Extra_Components", 1), ("C4MN_Extra_Value", 2), ("C4MN_Extra_MagicValue", 3), ("C4MN_Extra_Info", 4),
    ("C4MN_Extra_ComponentsMagic", 5), ("C4MN_Extra_LiveMagicValue", 6), ("C4MN_Extra_ComponentsLiveMagic", 7), ("C4MN_Add_ImgRank", 1), ("C4MN_Add_ImgIndexed", 2),
    ("C4MN_Add_ImgObjRank", 3), ("C4MN_Add_ImgObject", 4), ("C4MN_Add_ImgTextSpec", 5), ("C4MN_Add_ImgColor", 6), ("C4MN_Add_ImgIndexedColor", 7), ("C4MN_Add_PassValue", 128),
    ("C4MN_Add_ForceCount", 256), ("C4MN_Add_ForceNoDesc", 512), ("FX_OK", 0), ("FX_Effect_Deny", -1), ("FX_Effect_Annul", -2), ("FX_Effect_AnnulDoCalls", -3),
    ("FX_Execute_Kill", -1), ("FX_Stop_Deny", -1), ("FX_Start_Deny", -1), ("FX_Call_Normal", 0), ("FX_Call_Temp", 1), ("FX_Call_TempAddForRemoval", 2), ("FX_Call_RemoveClear", 3),
    ("FX_Call_RemoveDeath", 4), ("FX_Call_DmgScript", 0), ("FX_Call_DmgBlast", 1), ("FX_Call_DmgFire", 2), ("FX_Call_DmgChop", 3), ("FX_Call_Energy", 32),
    ("FX_Call_EngScript", 32), ("FX_Call_EngBlast", 33), ("FX_Call_EngObjHit", 34), ("FX_Call_EngFire", 35), ("FX_Call_EngBaseRefresh", 36), ("FX_Call_EngAsphyxiation", 37),
    ("FX_Call_EngCorrosion", 38), ("FX_Call_EngStruct", 39), ("FX_Call_EngGetPunched", 40), ("GFXOV_MODE_None", 0), ("GFXOV_MODE_Base", 1), ("GFXOV_MODE_Action", 2),
    ("GFXOV_MODE_Picture", 3), ("GFXOV_MODE_IngamePicture", 4), ("GFXOV_MODE_Object", 5), ("GFXOV_MODE_ExtraGraphics", 6), ("GFX_Overlay", 1), ("GFXOV_Clothing", 1000),
    ("GFXOV_Tools", 2000), ("GFXOV_ProcessTarget", 3000), ("GFXOV_Misc", 5000), ("GFXOV_UI", 6000), ("GFX_BLIT_Additive", 1), ("GFX_BLIT_Mod2", 2), ("GFX_BLIT_ClrSfc_OwnClr", 4),
    ("GFX_BLIT_ClrSfc_Mod2", 8), ("GFX_BLIT_Custom", 128), ("GFX_BLIT_Parent", 256), ("NO_OWNER", -1), ("CNAT_None", 0), ("CNAT_Left", 1), ("CNAT_Right", 2), ("CNAT_Top", 4),
    ("CNAT_Bottom", 8), ("CNAT_Center", 16), ("CNAT_MultiAttach", 32), ("CNAT_NoCollision", 64), ("VTX_X", 0), ("VTX_Y", 1), ("VTX_CNAT", 2), ("VTX_Friction", 3),
    ("VTX_SetPermanent", 1), ("VTX_SetPermanentUpd", 2), ("C4M_Vehicle", 100), ("C4M_Solid", 50), ("C4M_SemiSolid", 25), ("C4M_Liquid", 25), ("C4M_Background", 0),
    ("SBRD_Caption", -1), ("TEAM_Custom", 1), ("TEAM_Active", 2), ("TEAM_AllowHostilityChange", 3), ("TEAM_Dist", 4), ("TEAM_AllowTeamSwitch", 5), ("TEAM_AutoGenerateTeams", 6),
    ("TEAM_TeamColors", 7), ("C4OS_DELETED", 0), ("C4OS_NORMAL", 1), ("C4OS_INACTIVE", 2), ("C4MSGCMDR_Escaped", 0), ("C4MSGCMDR_Plain", 1), ("C4MSGCMDR_Identifier", 2),
    ("BASEFUNC_Default", 65535), ("BASEFUNC_AutoSellContents", 1), ("BASEFUNC_RegenerateEnergy", 2), ("BASEFUNC_Buy", 4), ("BASEFUNC_Sell", 8), ("BASEFUNC_RejectEntrance", 16),
    ("BASEFUNC_Extinguish", 32), ("C4FO_Not", 1), ("C4FO_And", 2), ("C4FO_Or", 3), ("C4FO_Exclude", 5), ("C4FO_InRect", 10), ("C4FO_AtPoint", 11), ("C4FO_AtRect", 12),
    ("C4FO_OnLine", 13), ("C4FO_Distance", 14), ("C4FO_ID", 20), ("C4FO_OCF", 21), ("C4FO_Category", 22), ("C4FO_Action", 30), ("C4FO_ActionTarget", 31), ("C4FO_Container", 40),
    ("C4FO_AnyContainer", 41), ("C4FO_Owner", 50), ("C4FO_Controller", 51), ("C4FO_Func", 60), ("C4FO_Layer", 70), ("C4SO_Reverse", 101), ("C4SO_Multiple", 102),
    ("C4SO_Distance", 110), ("C4SO_Random", 120), ("C4SO_Speed", 130), ("C4SO_Mass", 140), ("C4SO_Value", 150), ("C4SO_Func", 160), ("PHYS_Current", 0), ("PHYS_Permanent", 1),
    ("PHYS_Temporary", 2), ("PHYS_StackTemporary", 3), ("C4CMD_Base", 1), ("C4CMD_SilentBase", 2), ("C4CMD_Sub", 3), ("C4CMD_SilentSub", 0), ("C4CMD_MoveTo_NoPosAdjust", 1),
    ("C4CMD_MoveTo_PushTarget", 2), ("C4CMD_Enter_PushTarget", 2), ("C4SECT_SaveLandscape", 1), ("C4SECT_SaveObjects", 2), ("C4SECT_KeepEffects", 4), ("TEAMID_New", -1),
    ("MSG_NoLinebreak", 1), ("MSG_Bottom", 2), ("MSG_Multiple", 4), ("MSG_Top", 8), ("MSG_Left", 16), ("MSG_Right", 32), ("MSG_HCenter", 64), ("MSG_VCenter", 128),
    ("MSG_DropSpeech", 256), ("MSG_WidthRel", 512), ("MSG_XRel", 1024), ("MSG_YRel", 2048), ("MSG_ALeft", 4096), ("MSG_ACenter", 8192), ("MSG_ARight", 16384), ("C4PT_User", 1),
    ("C4PT_Script", 2), ("CSPF_FixedAttributes", 1), ("CSPF_NoScenarioInit", 2), ("CSPF_NoEliminationCheck", 4), ("CSPF_Invisible", 8), ("RESTORE_None", 0),
    ("RESTORE_ScriptPlayers", 1), ("RESTORE_PlayerTeams", 2), ("C4PVM_Cursor", 0), ("C4PVM_Target", 1), ("C4PVM_Scrolling", 2),
];

/// Registers the constant table on a script engine — the Rust analogue of
/// the RegisterGlobalConstant loop (C4Script.cpp:6580-6581).
pub(crate) fn register_script_constants(script: &mut ScriptEngine) {
    for (name, value) in C4_SCRIPT_CONSTANTS {
        script.register_constant(*name, Value::Int(*value));
    }
}

/// Seed the shared preparser table before any script host is loaded. C++ puts
/// built-ins and script-declared constants in the same engine map, so a
/// `static const` initializer may alias a built-in by name.
pub(crate) fn register_script_constants_in_global_table(constants: &clonk_script::GlobalVariables) {
    let mut constants = constants.borrow_mut();
    for (name, value) in C4_SCRIPT_CONSTANTS {
        constants.insert(
            (*name).to_owned(),
            clonk_script::value_cell(Value::Int(*value)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_the_cpp_entry_count_and_spot_values() {
        // 292 entries counted in C4Script.cpp:6210-6546.
        assert_eq!(C4_SCRIPT_CONSTANTS.len(), 292);
        let get = |name: &str| {
            C4_SCRIPT_CONSTANTS
                .iter()
                .find(|(entry, _)| *entry == name)
                .map(|(_, value)| *value)
        };
        assert_eq!(
            get("C4D_All"),
            Some(-1),
            "~C4D_None over int32 (C4Def.h:42)"
        );
        assert_eq!(
            get("OCF_Alive"),
            Some(i32::MIN),
            "1<<31 (C4Constants.h:137)"
        );
        assert_eq!(get("SkyPar_Keep"), Some(-163764), "C4Script.cpp:4948");
        assert_eq!(get("NO_OWNER"), Some(-1));
        assert_eq!(get("DIR_Right"), Some(1));
        assert_eq!(get("OCF_Chop"), Some(256));
        assert_eq!(
            get("C4D_GrabGet"),
            Some(2),
            "swapped vs name order (C4Def.h:80-81)"
        );
    }

    #[test]
    fn builtins_are_available_to_static_const_initializers() {
        let globals = clonk_script::new_global_variables();
        let constants = clonk_script::new_global_variables();
        register_script_constants_in_global_table(&constants);
        let script =
            clonk_script::Script::compile("#strict 3\nstatic const RIGHT_ALIAS = DIR_Right;")
                .expect("static constant alias compiles");

        clonk_script::register_global_declarations(script.var_decls(), &globals, Some(&constants))
            .expect("built-in constant resolves during preparse");

        assert_eq!(
            constants
                .borrow()
                .get("RIGHT_ALIAS")
                .expect("alias registered")
                .borrow()
                .clone(),
            Value::Int(1)
        );

        let mut host = ScriptEngine::new();
        host.set_global_constants(constants.clone());
        register_script_constants(&mut host);
        host.add_script(
            clonk_script::Script::compile(
                "#strict\nfunc Probe() { return [DIR_Right + 0, DIR_Right()]; }",
            )
            .expect("built-in read probe compiles"),
        );
        *constants
            .borrow()
            .get("DIR_Right")
            .expect("built-in shared cell exists")
            .borrow_mut() = Value::Int(9);
        assert_eq!(
            host.call("Probe", &[])
                .expect("overridden constant resolves"),
            Value::Array(vec![Value::Int(9), Value::Int(9)]),
            "the canonical shared table wins over each host's stale built-in copy"
        );
    }
}
