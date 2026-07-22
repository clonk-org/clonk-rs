use super::*;


/// Native functions hidden from `C4Console::UpdateInputCtrl` by C++
/// `GetPublic() == false`. Most use `AddFunc`'s final `false`; the cast helpers
/// hard-code the same visibility. Rust's VM does not otherwise need native
/// visibility metadata, so keep this console-only projection by registration.
const NON_PUBLIC_CONSOLE_HOST_FUNCTIONS: &[&str] = &[
    "AssignVar",
    "ScoreboardCol",
    "CastInt",
    "CastBool",
    "CastC4ID",
    "CastAny",
    "Call",
    "Or",
    "Not",
    "And",
    "BitAnd",
    "Sum",
    "Sub",
    "Mul",
    "Div",
    "Mod",
    "Pow",
    "LessThan",
    "GreaterThan",
    "SEqual",
    "SetContactDensity",
    "ObjectSetAction",
    "SoundLevel",
    "SetCrewStatus",
    "DrawVolcanoBranch",
    "FlameConsumeMaterial",
    "TestMessageBoard",
    "CallMessageBoard",
    "AbortMessageBoard",
    "OnMessageBoardAnswer",
    "GetSystemTime",
    "IsNewgfx",
    "GetObjectLayer",
    "SetObjectLayer",
    "SetGameSpeed",
    "DrawMatChunks",
    "SetTextureIndex",
    "RemoveUnusedTexMapEntries",
    "SetObjDrawTransform2",
    "LoadScenarioSection",
    "SetObjectStatus",
    "GetObjectStatus",
    "AdjustWalkRotation",
    "FxFireStart",
    "FxFireTimer",
    "FxFireStop",
    "FxFireInfo",
    "SetPreSend",
    "GetPlayerID",
    "InitScenarioPlayer",
    "OnOwnerRemoved",
    "SetScoreboardData",
    "GetScoreboardString",
    "GetScoreboardData",
    "DoScoreboardShow",
    "SortScoreboard",
    "AddEvaluationData",
    "GetLeagueScore",
    "HideSettlementScoreInEvaluation",
    "GetUnusedOverlayID",
    "FatalError",
];

/// C++ registrations whose callback receives the converted `C4Value` array
/// itself instead of passing through `C4ValueConv<Par>::_FromC4V`.
pub(crate) const RAW_CPP_NATIVE_FUNCTIONS: &[&str] = &[
    "CastAny",
    "CastBool",
    "CastC4ID",
    "CastInt",
    "FindObject2",
    "FindObjects",
    "ObjectCount2",
    "ScoreboardCol",
];

/// AddFunc registrations backed by Rust's reference-aware callback surface.
/// Removing one of these as though it were an ordinary callback would discard
/// its `HostCallArg` provenance before signatures are attached below.
pub(crate) const REFERENCE_AWARE_CPP_NATIVE_FUNCTIONS: &[&str] = &[
    "Dec",
    "Equal",
    "GetIndexOf",
    "Inc",
    "PathFree2",
    "Set",
    "SetLength",
    "SimFlight",
];

/// `C4ValueConv<std::optional<T>>` retains C4V_Any as `nullopt` even though
/// its declared native type is indistinguishable from `T` in the signature.
const OPTIONAL_CPP_NATIVE_PARAMETER_SLOTS: &[(&str, usize)] =
    &[("CustomMessage", 5), ("ModulateColor", 0)];

fn is_optional_cpp_native_parameter(name: &str, index: usize) -> bool {
    OPTIONAL_CPP_NATIVE_PARAMETER_SLOTS.contains(&(name, index))
}

/// Mirror `C4ValueConv<bool>::_FromC4V`: unlike script truthiness this reads
/// the low `Data.Int` word. Rust models live pointer values semantically, so
/// every non-null string/array/map/object pointer is truthy.
pub(crate) fn extract_cpp_native_bool(value: &Value) -> bool {
    match value {
        Value::Int(value) => *value != 0,
        Value::Bool(value) => *value,
        Value::RawBool(value) => (*value as u32 as i32) != 0,
        Value::C4Id(value) => (clonk_script::c4_id_raw(value) as u32 as i32) != 0,
        Value::Object(value) => *value != 0,
        Value::String(_) | Value::Array(_) | Value::Proplist(_) => true,
        Value::Nil => false,
    }
}

fn cpp_native_extraction_error(
    name: &str,
    index: usize,
    expected: C4VType,
    value: &Value,
) -> RuntimeError {
    RuntimeError::new(format!(
        "{name}: VM admitted {} for native parameter {index}, expected {expected:?}",
        value.type_name()
    ))
}

/// Canonicalize one VM-checked AddFunc argument exactly as the corresponding
/// `C4ValueConv<Par>::_FromC4V` extraction followed by a Rust `Value` bridge.
pub(crate) fn extract_cpp_native_argument(
    name: &str,
    index: usize,
    expected: C4VType,
    value: &Value,
) -> Result<Value, RuntimeError> {
    if value == &Value::Nil && is_optional_cpp_native_parameter(name, index) {
        return Ok(Value::Nil);
    }

    match expected {
        C4VType::Any => Ok(value.clone()),
        C4VType::Int => value
            .as_c4_int()
            .map(Value::Int)
            .ok_or_else(|| cpp_native_extraction_error(name, index, expected, value)),
        C4VType::Bool => Ok(Value::Bool(extract_cpp_native_bool(value))),
        C4VType::C4Id => match value {
            Value::Nil => Ok(Value::Nil),
            Value::C4Id(value) => {
                let raw = clonk_script::c4_id_raw(value);
                Ok(if raw == 0 {
                    Value::Nil
                } else {
                    Value::C4Id(clonk_script::c4_id_from_raw(raw))
                })
            }
            _ => Err(cpp_native_extraction_error(name, index, expected, value)),
        },
        C4VType::C4Object => match value {
            Value::Nil | Value::Object(0) => Ok(Value::Nil),
            Value::Object(value) => Ok(Value::Object(*value)),
            _ => Err(cpp_native_extraction_error(name, index, expected, value)),
        },
        C4VType::String => match value {
            Value::Nil => Ok(Value::Nil),
            Value::String(value) => Ok(Value::String(value.clone())),
            _ => Err(cpp_native_extraction_error(name, index, expected, value)),
        },
        C4VType::Array => match value {
            Value::Nil => Ok(Value::Nil),
            Value::Array(value) => Ok(Value::Array(value.clone())),
            _ => Err(cpp_native_extraction_error(name, index, expected, value)),
        },
        C4VType::Map => match value {
            Value::Nil => Ok(Value::Nil),
            Value::Proplist(value) => Ok(Value::Proplist(value.clone())),
            _ => Err(cpp_native_extraction_error(name, index, expected, value)),
        },
        C4VType::Ref => Err(cpp_native_extraction_error(name, index, expected, value)),
    }
}

pub(crate) fn extract_cpp_native_arguments(
    name: &str,
    parameter_types: &[C4VType],
    args: &[Value],
) -> Result<Vec<Value>, RuntimeError> {
    args.iter()
        .enumerate()
        .map(|(index, value)| match parameter_types.get(index) {
            Some(expected) => extract_cpp_native_argument(name, index, *expected, value),
            // EffectVar carries a private fourth setter value beyond its
            // three public C++ parameters. Preserve all such private tails.
            None => Ok(value.clone()),
        })
        .collect()
}

pub(crate) fn wrap_cpp_add_func_host_function(
    script: &mut ScriptEngine,
    name: &'static str,
    parameter_types: &'static [C4VType],
) {
    let parameter_count = script
        .host_function_parameter_count(name)
        .unwrap_or(parameter_types.len());
    assert_eq!(
        parameter_count,
        parameter_types.len(),
        "native extractor signature must match the registered arity for {name}"
    );
    let callback = script
        .remove_host_function(name)
        .unwrap_or_else(|| panic!("ordinary AddFunc callback is not registered: {name}"));
    script.register_host_function_with_arity(name, parameter_count, move |args| {
        let extracted = extract_cpp_native_arguments(name, parameter_types, args)?;
        callback(&extracted)
    });
}

fn install_cpp_add_func_argument_extractors(script: &mut ScriptEngine) {
    let mut add_func_count = 0;
    let mut wrapped_count = 0;

    for (name, parameter_types) in
        crate::native_function_parameters::native_function_parameter_entries()
    {
        if crate::native_function_parameters::RUST_STANDIN_NATIVE_FUNCTIONS.contains(&name)
            || RAW_CPP_NATIVE_FUNCTIONS.contains(&name)
        {
            continue;
        }
        add_func_count += 1;
        if REFERENCE_AWARE_CPP_NATIVE_FUNCTIONS.contains(&name) {
            continue;
        }
        wrap_cpp_add_func_host_function(script, name, parameter_types);
        wrapped_count += 1;
    }

    let expected_add_func_count =
        crate::native_function_parameters::CPP_BACKED_NATIVE_FUNCTION_COUNT
            - RAW_CPP_NATIVE_FUNCTIONS.len();
    assert_eq!(add_func_count, expected_add_func_count);
    assert_eq!(
        wrapped_count + REFERENCE_AWARE_CPP_NATIVE_FUNCTIONS.len(),
        expected_add_func_count
    );
}

pub(crate) fn public_console_host_function_names(script: &ScriptEngine) -> Vec<String> {
    script
        .host_function_names()
        .into_iter()
        .filter(|name| !NON_PUBLIC_CONSOLE_HOST_FUNCTIONS.contains(&name.as_str()))
        .collect()
}

/// Registers the engine's C++-backed native surface without letting a call
/// site omit its oracle-declared parameter count. The raw callbacks remain
/// untouched because EffectVar uses a private fourth argument for retained
/// lvalue writes; the script VM applies this metadata only at public calls.
struct CppNativeHostRegistrar<'a> {
    engine: &'a mut ScriptEngine,
}

/// Exact `GetParCount()` values from C++ `InitFunctionMap`. The five
/// System.c4g fallbacks keep ten slots because C4Aul script functions inherit
/// the base `C4AUL_MAX_Par` count; the two Rust-only helpers use their public
/// compatibility signatures.
pub(crate) fn cpp_native_parameter_count(name: &str) -> usize {
    match name {
        "AnyContainer"
        | "DeathAnnounce"
        | "EditCursor"
        | "FrameCounter"
        | "GetClimate"
        | "GetGravity"
        | "GetMatAdjust"
        | "GetMaxPlayer"
        | "GetSeason"
        | "GetTeamCount"
        | "GetTemperature"
        | "GetTime"
        | "IsNetwork"
        | "IsNewgfx"
        | "LandscapeHeight"
        | "LandscapeWidth"
        | "NoContainer"
        | "RemoveUnusedTexMapEntries"
        | "ScriptCounter"
        | "StartCallTrace"
        | "StopScriptProfiler" => 0,
        "Abs"
        | "ActIdle"
        | "ActivateGameGoalMenu"
        | "AsyncRandom"
        | "C4Id"
        | "CastAny"
        | "CastBool"
        | "CastC4ID"
        | "CastInt"
        | "CheckEnergyNeedChain"
        | "ClearLastPlrCom"
        | "ClearMenuItems"
        | "CloseMenu"
        | "Contained"
        | "CreateArray"
        | "CrewMember"
        | "DecVar"
        | "ExecuteCommand"
        | "Extinguish"
        | "FatalError"
        | "GainMissionAccess"
        | "GameOver"
        | "GetActTime"
        | "GetAction"
        | "GetActionData"
        | "GetAlive"
        | "GetBase"
        | "GetBreath"
        | "GetCaptain"
        | "GetColor"
        | "GetColorDw"
        | "GetComDir"
        | "GetCon"
        | "GetController"
        | "GetCrewCount"
        | "GetCrewEnabled"
        | "GetDamage"
        | "GetDefBottom"
        | "GetDir"
        | "GetEnergy"
        | "GetEntrance"
        | "GetHiRank"
        | "GetID"
        | "GetKeys"
        | "GetKiller"
        | "GetLeague"
        | "GetLeagueProgressData"
        | "GetLeagueScore"
        | "GetLength"
        | "GetMagicEnergy"
        | "GetMenu"
        | "GetMenuSelection"
        | "GetMissionAccess"
        | "GetNeededMatStr"
        | "GetOCF"
        | "GetObjectLayer"
        | "GetObjectStatus"
        | "GetOwner"
        | "GetPhase"
        | "GetPlayerCount"
        | "GetPlayerID"
        | "GetPlayerName"
        | "GetPlayerTeam"
        | "GetPlayerType"
        | "GetPlrColorDw"
        | "GetPlrDownDouble"
        | "GetPlrJumpAndRunControl"
        | "GetPlrValue"
        | "GetPlrValueGain"
        | "GetPlrView"
        | "GetPlrViewMode"
        | "GetProcedure"
        | "GetR"
        | "GetRank"
        | "GetScore"
        | "GetSelectCount"
        | "GetSkyAdjust"
        | "GetSystemTime"
        | "GetTaggedPlayerName"
        | "GetTeamByIndex"
        | "GetTeamColor"
        | "GetTeamConfig"
        | "GetTeamName"
        | "GetType"
        | "GetValues"
        | "GetVertexNum"
        | "GetViewCursor"
        | "GetVisibility"
        | "GetWealth"
        | "GetX"
        | "GetY"
        | "HideSettlementScoreInEvaluation"
        | "InLiquid"
        | "IncVar"
        | "Incinerate"
        | "IsRef"
        | "Jump"
        | "LaunchVolcano"
        | "Material"
        | "MaterialName"
        | "MusicLevel"
        | "Not"
        | "Object"
        | "ObjectNumber"
        | "OnFire"
        | "PauseGame"
        | "PlaceAnimal"
        | "Random"
        | "ReloadDef"
        | "ReloadParticle"
        | "ResetGamma"
        | "Resort"
        | "ScoreboardCol"
        | "ScriptGo"
        | "ScrollContents"
        | "SetClimate"
        | "SetFilmView"
        | "SetGameSpeed"
        | "SetGravity"
        | "SetMatAdjust"
        | "SetMaxPlayer"
        | "SetRestoreInfos"
        | "SetSeason"
        | "SetTemperature"
        | "SetWind"
        | "ShowInfo"
        | "Split2Components"
        | "Sqrt"
        | "StartScriptProfiler"
        | "Stuck"
        | "SurrenderPlayer"
        | "UnselectCrew"
        | "Value"
        | "goto" => 1,
        "AbortMessageBoard"
        | "AddEvaluationData"
        | "And"
        | "ArcCos"
        | "ArcSin"
        | "AssignVar"
        | "BitAnd"
        | "Bubble"
        | "ChangeDef"
        | "ClearParticles"
        | "Collect"
        | "ComponentAll"
        | "ComposeContents"
        | "ContentsCount"
        | "Dec"
        | "Div"
        | "DoBreath"
        | "DoCon"
        | "DoCrewExp"
        | "DoScore"
        | "DoScoreboardShow"
        | "EliminatePlayer"
        | "EnergyCheck"
        | "Enter"
        | "Equal"
        | "ExtractLiquid"
        | "FightWith"
        | "FindBase"
        | "FindContents"
        | "FindOtherContents"
        | "FlameConsumeMaterial"
        | "FxFireInfo"
        | "GBackLiquid"
        | "GBackSemiSolid"
        | "GBackSky"
        | "GBackSolid"
        | "GetActionTarget"
        | "GetCategory"
        | "GetChar"
        | "GetClrModulation"
        | "GetCrew"
        | "GetCrewExtraData"
        | "GetCursor"
        | "GetDefinition"
        | "GetDesc"
        | "GetIndexOf"
        | "GetMass"
        | "GetMaterial"
        | "GetMaterialCount"
        | "GetName"
        | "GetObjectBlitMode"
        | "GetPlayerByIndex"
        | "GetPlrExtraData"
        | "GetRDir"
        | "GetScoreboardData"
        | "GetScoreboardString"
        | "GetSkyColor"
        | "GetTexture"
        | "GetUnusedOverlayID"
        | "GetXDir"
        | "GetYDir"
        | "GrabContents"
        | "GrabObjectInfo"
        | "GreaterThan"
        | "Inc"
        | "IncinerateLandscape"
        | "InitScenarioPlayer"
        | "Kill"
        | "LaunchEarthquake"
        | "LessThan"
        | "LoadScenarioSection"
        | "MakeCrewMember"
        | "Max"
        | "Min"
        | "Mod"
        | "ModulateColor"
        | "Mul"
        | "Music"
        | "ObjectDistance"
        | "Pow"
        | "Punch"
        | "RemoveObject"
        | "RemoveVertex"
        | "ResetPhysical"
        | "ResortObject"
        | "ResortObjects"
        | "SEqual"
        | "SelectMenuItem"
        | "Sell"
        | "Set"
        | "SetActionData"
        | "SetAlive"
        | "SetCategory"
        | "SetColor"
        | "SetColorDw"
        | "SetComDir"
        | "SetContactDensity"
        | "SetController"
        | "SetCrewEnabled"
        | "SetDir"
        | "SetEntrance"
        | "SetFoW"
        | "SetKiller"
        | "SetLeaguePerformance"
        | "SetLeagueProgressData"
        | "SetLength"
        | "SetMass"
        | "SetMenuDecoration"
        | "SetMenuTextProgress"
        | "SetObjectLayer"
        | "SetOwner"
        | "SetPhase"
        | "SetPlayList"
        | "SetPlrShowCommand"
        | "SetPlrShowControl"
        | "SetPlrShowControlPos"
        | "SetPlrView"
        | "SetPreSend"
        | "SetR"
        | "SetSkyAdjust"
        | "SetVar"
        | "SetViewCursor"
        | "SetVisibility"
        | "SetWealth"
        | "SortScoreboard"
        | "TestMessageBoard"
        | "WildcardMatch" => 2,
        "AddMsgBoardCmd"
        | "AddVertex"
        | "BlastObject"
        | "BoundBy"
        | "Contents"
        | "Cos"
        | "CreateContents"
        | "DoHomebaseMaterial"
        | "DoHomebaseProduction"
        | "DoMagicEnergy"
        | "EffectVar"
        | "FindConstructionSite"
        | "FinishCommand"
        | "FxFireTimer"
        | "GetCommand"
        | "GetContact"
        | "GetEffectCount"
        | "GetMaterialColor"
        | "GetPlrControlName"
        | "GetPlrMagic"
        | "GetPortrait"
        | "GetScenarioVal"
        | "GetVertex"
        | "GetVertexContact"
        | "GetWind"
        | "Hostile"
        | "Inside"
        | "LocateFunc"
        | "OnMessageBoardAnswer"
        | "PushParticles"
        | "SetActionTargets"
        | "SetClrModulation"
        | "SetComponent"
        | "SetCrewExtraData"
        | "SetCrewStatus"
        | "SetLandscapePixel"
        | "SetMenuSize"
        | "SetNextMission"
        | "SetObjectBlitMode"
        | "SetObjectOrder"
        | "SetObjectStatus"
        | "SetPlayerTeam"
        | "SetPlrExtraData"
        | "SetPlrKnowledge"
        | "SetPlrMagic"
        | "SetPlrViewRange"
        | "SetRDir"
        | "SetTextureIndex"
        | "SetViewOffset"
        | "SetXDir"
        | "SetYDir"
        | "ShakeFree"
        | "ShakeObjects"
        | "Sin"
        | "SoundLevel" => 3,
        "AdjustWalkRotation"
        | "BlastFree"
        | "CallMessageBoard"
        | "CreateObject"
        | "DigFree"
        | "Distance"
        | "DoDamage"
        | "Explode"
        | "ExtractMaterialAmount"
        | "FxFireStop"
        | "GetActMapVal"
        | "GetComponent"
        | "GetDefCoreVal"
        | "GetHomebaseMaterial"
        | "GetHomebaseProduction"
        | "GetMaterialVal"
        | "GetObjectInfoCoreVal"
        | "GetObjectVal"
        | "GetPath"
        | "GetPhysical"
        | "GetPlayerInfoCoreVal"
        | "GetPlayerVal"
        | "GetPlrKnowledge"
        | "GetValue"
        | "PathFree"
        | "PathFree2"
        | "RemoveEffect"
        | "SelectCrew"
        | "SetAction"
        | "SetGamma"
        | "SetPhysical"
        | "SetPosition"
        | "SetScoreboardData"
        | "SetSkyColor"
        | "ShiftContents"
        | "Smoke"
        | "Sub"
        | "Sum"
        | "TrainPhysical" => 4,
        "Angle"
        | "BlastObjects"
        | "Buy"
        | "CastObjects"
        | "CastPXS"
        | "ChangeEffect"
        | "CreateScriptPlayer"
        | "DigFreeRect"
        | "DoEnergy"
        | "DrawDefMap"
        | "DrawMap"
        | "Fling"
        | "FreeRect"
        | "GetEffect"
        | "InsertMaterial"
        | "ObjectSetAction"
        | "Or"
        | "SetBridgeActionData"
        | "SetCursor"
        | "SetHostility"
        | "SetName"
        | "SetPicture"
        | "SetPortrait"
        | "SetShape"
        | "SetTransferZone"
        | "SetVertex" => 5,
        "DrawVolcanoBranch" | "FxFireStart" | "PlaceVegetation" | "SetSkyFade" => 6,
        "CreateConstruction"
        | "Exit"
        | "LaunchLightning"
        | "PlayerObjectCommand"
        | "SetSkyParallax"
        | "SetSolidMask" => 7,
        "CheckEffect"
        | "SetCommand"
        | "SetGraphics"
        | "SetObjDrawTransform"
        | "SimFlight"
        | "Sound" => 8,
        "CreateMenu" | "CreateParticle" | "DrawMatChunks" => 9,
        "AddCommand"
        | "AddEffect"
        | "AddMenuItem"
        | "AddMessage"
        | "AppendCommand"
        | "Call"
        | "CastBackParticles"
        | "CastParticles"
        | "CustomMessage"
        | "DebugLog"
        | "DefinitionCall"
        | "DrawMaterialQuad"
        | "EffectCall"
        | "FindObject"
        | "FindObject2"
        | "FindObjectOwner"
        | "FindObjects"
        | "Find_AtPoint"
        | "Find_Category"
        | "Find_ID"
        | "Format"
        | "GameCall"
        | "GameCallEx"
        | "GetObjHeight"
        | "GetObjWidth"
        | "Log"
        | "Message"
        | "ObjectCall"
        | "ObjectCount"
        | "ObjectCount2"
        | "PlayerMessage"
        | "PlrMessage"
        | "PrivateCall"
        | "ProtectedCall"
        | "SetMaterialColor"
        | "SetObjDrawTransform2" => 10,
        _ => panic!("missing C++ native arity for {name}"),
    }
}

impl CppNativeHostRegistrar<'_> {
    fn register_host_function<F>(&mut self, name: &'static str, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        self.engine
            .register_host_function_with_arity(name, cpp_native_parameter_count(name), func);
    }

    fn register_host_reference_function<F, I>(
        &mut self,
        name: &'static str,
        reference_parameters: I,
        func: F,
    ) where
        F: Fn(&[clonk_script::HostCallArg]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
        I: IntoIterator<Item = usize>,
    {
        self.engine.register_host_reference_function_with_arity(
            name,
            cpp_native_parameter_count(name),
            reference_parameters,
            func,
        );
    }
}

fn populate_host_registration_template(script: &mut ScriptEngine) {
    // Every script host knows the engine constant table
    // (RegisterGlobalConstant, C4Script.cpp:6580-6581).
    crate::script_constants::register_script_constants(script);
    let mut registrar = CppNativeHostRegistrar { engine: script };
    let script = &mut registrar;
    script.register_host_function("AddEffect", add_effect);
    script.register_host_function("CheckEffect", check_effect);
    script.register_host_function("ChangeEffect", change_effect);
    script.register_host_function("RemoveEffect", remove_effect);
    script.register_host_function("GetEffect", get_effect);
    script.register_host_function("GetEffectCount", get_effect_count);
    script.register_host_function("EffectCall", effect_call);
    script.register_host_function("WildcardMatch", wildcard_match);
    script.register_host_function("EffectVar", effect_var);
    script.register_host_function("IsNetwork", is_network);
    script.register_host_function("GetPlayerCount", get_player_count);
    script.register_host_function("GetPlayerByIndex", get_player_by_index);
    script.register_host_function("CreateScriptPlayer", create_script_player);
    script.register_host_function("InitScenarioPlayer", init_scenario_player);
    script.register_host_function("ActivateGameGoalMenu", activate_game_goal_menu);
    script.register_host_function("EliminatePlayer", eliminate_player);
    script.register_host_function("SurrenderPlayer", surrender_player);
    script.register_host_function("GetPlayerName", get_player_name);
    script.register_host_function("GetTaggedPlayerName", get_tagged_player_name);
    script.register_host_function("GetPlayerVal", get_player_val);
    script.register_host_function("GetPlayerInfoCoreVal", get_player_info_core_val);
    script.register_host_function("GetPlayerTeam", get_player_team);
    script.register_host_function("SetPlayerTeam", set_player_team);
    script.register_host_function("GetMaxPlayer", get_max_player);
    script.register_host_function("SetMaxPlayer", set_max_player);
    script.register_host_function("TestMessageBoard", test_message_board);
    script.register_host_function("CallMessageBoard", call_message_board);
    script.register_host_function("AbortMessageBoard", abort_message_board);
    script.register_host_function("OnMessageBoardAnswer", on_message_board_answer);
    script.register_host_function("AddMsgBoardCmd", add_msg_board_cmd);
    script.register_host_function("GetTeamConfig", get_team_config);
    script.register_host_function("GetTeamCount", get_team_count);
    script.register_host_function("GetTeamByIndex", get_team_by_index);
    script.register_host_function("GetTeamColor", get_team_color);
    script.register_host_function("GetTeamName", get_team_name);
    script.register_host_function("GetPlayerType", get_player_type);
    script.register_host_function("GetPlayerID", get_player_id);
    script.register_host_function("GetWealth", get_wealth);
    script.register_host_function("SetWealth", set_wealth);
    script.register_host_function("Hostile", hostile);
    script.register_host_function("SetHostility", set_hostility);
    script.register_host_function("SetFoW", set_fow);
    script.register_host_function("SetScoreboardData", set_scoreboard_data);
    // Player and crew C4ValueMapData functions (C4Script.cpp:4692-4800,
    // AddFunc :6660-6663).
    script.register_host_function("GetPlrExtraData", get_plr_extra_data);
    script.register_host_function("SetPlrExtraData", set_plr_extra_data);
    script.register_host_function("GetCrewExtraData", get_crew_extra_data);
    script.register_host_function("SetCrewExtraData", set_crew_extra_data);
    script.register_host_function("GetScenarioVal", get_scenario_val);
    script.register_host_function("LoadScenarioSection", load_scenario_section);
    script.register_host_function("GetLeague", get_league);
    script.register_host_function("GetLeagueProgressData", get_league_progress_data);
    script.register_host_function("GetLeagueScore", get_league_score);
    script.register_host_function("SetLeagueProgressData", set_league_progress_data);
    script.register_host_function("SetLeaguePerformance", set_league_performance);
    script.register_host_function("DoScore", do_score);
    script.register_host_function("DoCrewExp", do_crew_exp);
    script.register_host_function("ReloadDef", reload_def);
    script.register_host_function("ReloadParticle", reload_particle);
    script.register_host_function("PauseGame", pause_game);
    script.register_host_function("GetScore", get_score);
    script.register_host_function("GetScoreboardString", get_scoreboard_string);
    script.register_host_function("GetScoreboardData", get_scoreboard_data);
    script.register_host_function("GetPlrValue", get_plr_value);
    script.register_host_function("GetPlrValueGain", get_plr_value_gain);
    script.register_host_function("GetPlrKnowledge", get_plr_knowledge);
    script.register_host_function("GetPlrMagic", get_plr_magic);
    script.register_host_function("GetCrew", get_crew);
    script.register_host_function("GetHiRank", get_hi_rank);
    script.register_host_function("GetRank", get_rank);
    script.register_host_function("SetComponent", set_component);
    script.register_host_function("GetDefinition", get_definition);
    script.register_host_function("Value", definition_value);
    script.register_host_function("GetValue", get_value);
    script.register_host_function("Buy", buy);
    script.register_host_function("Sell", sell);
    script.register_host_function("GetDefCoreVal", get_def_core_val);
    script.register_host_function("Enter", enter);
    script.register_host_function("Collect", collect);
    script.register_host_function("Exit", exit_container);
    script.register_host_function("GetComponent", get_component);
    script.register_host_function("GetNeededMatStr", get_needed_mat_str);
    script.register_host_function("ComponentAll", component_all);
    script.register_host_function("GrabContents", grab_contents);
    script.register_host_function("InLiquid", in_liquid);
    script.register_host_function("Material", material);
    script.register_host_function("MaterialName", material_name);
    script.register_host_function("GetMaterialCount", get_material_count);
    script.register_host_function("GetMaterialColor", get_material_color);
    script.register_host_function("SetMaterialColor", set_material_color);
    script.register_host_function("GetMaterialVal", get_material_val);
    script.register_host_function("ObjectSetAction", object_set_action);
    script.register_host_function("Smoke", smoke);
    script.register_host_function("Bubble", bubble);
    script.register_host_reference_function("SimFlight", 0..4, sim_flight);
    script.register_host_function("SetPortrait", set_portrait);
    script.register_host_function("GetPortrait", get_portrait);
    script.register_host_function("SetVisibility", set_visibility);
    script.register_host_function("SetPlrViewRange", set_plr_view_range);
    script.register_host_function("AddMenuItem", add_menu_item);
    script.register_host_function("ClearMenuItems", clear_menu_items);
    script.register_host_function("CloseMenu", close_menu);
    script.register_host_function("CreateMenu", create_menu);
    script.register_host_function("GetMenu", get_menu);
    script.register_host_function("ShowInfo", show_info);
    script.register_host_function("GetMenuSelection", get_menu_selection);
    script.register_host_function("SelectMenuItem", select_menu_item);
    script.register_host_function("SetMenuDecoration", set_menu_decoration);
    script.register_host_function("SetMenuSize", set_menu_size);
    script.register_host_function("SetMenuTextProgress", set_menu_text_progress);
    script.register_host_function("SetPlrView", set_plr_view);
    script.register_host_function("GetPlrViewMode", get_plr_view_mode);
    script.register_host_function("GetPlrView", get_plr_view);
    script.register_host_function("SetFilmView", set_film_view);
    script.register_host_function("SetGameSpeed", set_game_speed);
    script.register_host_function("SetPreSend", set_pre_send);
    script.register_host_function("FrameCounter", frame_counter);
    script.register_host_function("GetTime", get_time);
    script.register_host_function("GetSystemTime", get_system_time);
    script.register_host_function("LandscapeWidth", landscape_width);
    script.register_host_function("LandscapeHeight", landscape_height);
    script.register_host_function("LaunchLightning", launch_lightning);
    script.register_host_function("LaunchVolcano", launch_volcano);
    script.register_host_function("LaunchEarthquake", launch_earthquake);
    script.register_host_function("SetSolidMask", set_solid_mask);
    script.register_host_function("ChangeDef", change_def);
    script.register_host_function("GetPlrDownDouble", get_plr_down_double);
    script.register_host_function("ClearLastPlrCom", clear_last_plr_com);
    script.register_host_function("SetClrModulation", set_clr_modulation);
    script.register_host_function("GetClrModulation", get_clr_modulation);
    script.register_host_function("ModulateColor", modulate_color);
    script.register_host_function("GetCrewCount", get_crew_count);
    script.register_host_function("GetCursor", get_cursor_host);
    script.register_host_function("SelectCrew", select_crew_host);
    script.register_host_function("SetCrewStatus", set_crew_status);
    script.register_host_function("UnselectCrew", unselect_crew_host);
    script.register_host_function("GetViewCursor", get_view_cursor);
    script.register_host_function("EditCursor", edit_cursor);
    script.register_host_function("GetCaptain", get_captain);
    script.register_host_function("SetViewCursor", set_view_cursor);
    script.register_host_function("GetSelectCount", get_select_count);
    script.register_host_function("SetPlrKnowledge", set_plr_knowledge);
    script.register_host_function("SetPlrMagic", set_plr_magic);
    script.register_host_function("SetPlrShowControl", set_plr_show_control);
    script.register_host_function("SetPlrShowCommand", set_plr_show_command);
    script.register_host_function("SetPlrShowControlPos", set_plr_show_control_pos);
    script.register_host_function("SetAction", set_action);
    script.register_host_function("SetBridgeActionData", set_bridge_action_data);
    script.register_host_function("SetActionData", set_action_data);
    script.register_host_function("GetActionData", get_action_data);
    script.register_host_function("GetAction", get_action);
    script.register_host_function("GetCommand", get_command);
    script.register_host_function("PlayerObjectCommand", player_object_command_host);
    script.register_host_function("ShiftContents", shift_contents);
    script.register_host_function("ScrollContents", scroll_contents);
    script.register_host_function("GetActTime", get_act_time);
    script.register_host_function("GetPhase", get_phase);
    script.register_host_function("SetPhase", set_phase);
    script.register_host_function("GetProcedure", get_procedure);
    script.register_host_function("SetActionTargets", set_action_targets);
    script.register_host_function("GetActionTarget", get_action_target);
    script.register_host_function("GetVertexNum", get_vertex_num);
    script.register_host_function("GetVertex", get_vertex);
    script.register_host_function("GetVertexContact", get_vertex_contact);
    script.register_host_function("Stuck", stuck);
    script.register_host_function("Inside", inside);
    script.register_host_function("GetVisibility", get_visibility);
    script.register_host_function("FinishCommand", finish_command);
    script.register_host_function("SetCrewEnabled", set_crew_enabled);
    script.register_host_function("GetCrewEnabled", get_crew_enabled);
    script.register_host_function("GetChar", get_char);
    script.register_host_function("GetColor", get_color);
    script.register_host_function("GetColorDw", get_color_dw);
    script.register_host_function("SetCursor", set_cursor_host);
    script.register_host_function("Fling", fling);
    script.register_host_function("Jump", jump);
    script.register_host_function("Kill", kill);
    script.register_host_function("Punch", punch);
    script.register_host_function("EnergyCheck", energy_check);
    script.register_host_function("CheckEnergyNeedChain", check_energy_need_chain);
    script.register_host_function("GetContact", get_contact);
    script.register_host_function("PathFree", path_free);
    script.register_host_reference_function("PathFree2", 0..2, path_free2);
    script.register_host_function("GetPath", get_path);
    script.register_host_function("SetTransferZone", set_transfer_zone);
    script.register_host_function("DigFree", dig_free);
    script.register_host_function("DigFreeRect", dig_free_rect);
    script.register_host_function("FreeRect", free_rect);
    script.register_host_function("DrawDefMap", draw_def_map);
    script.register_host_function("DrawMap", draw_map);
    script.register_host_function("DrawMatChunks", draw_mat_chunks);
    script.register_host_function("DrawMaterialQuad", draw_material_quad);
    script.register_host_function("DrawVolcanoBranch", draw_volcano_branch);
    script.register_host_function("ScriptGo", script_go);
    script.register_host_function("ScriptCounter", script_counter);
    script.register_host_function("goto", script_goto);
    script.register_host_function("BlastFree", blast_free);
    script.register_host_function("BlastObject", blast_object);
    script.register_host_function("BlastObjects", blast_objects);
    script.register_host_function("Explode", explode);
    script.register_host_function("ShakeFree", shake_free);
    script.register_host_function("ShakeObjects", shake_objects);
    script.register_host_function("SetSkyParallax", set_sky_parallax);
    script.register_host_function("SetSkyAdjust", set_sky_adjust);
    script.register_host_function("SetSkyColor", set_sky_color);
    script.register_host_function("SetSkyFade", set_sky_fade);
    script.register_host_function("SetMatAdjust", set_mat_adjust);
    script.register_host_function("GetMatAdjust", get_mat_adjust);
    script.register_host_function("SetLandscapePixel", set_landscape_pixel);
    script.register_host_function("SetTextureIndex", set_texture_index);
    script.register_host_function(
        "RemoveUnusedTexMapEntries",
        remove_unused_texmap_entries,
    );
    script.register_host_function("GetSkyAdjust", get_sky_adjust);
    script.register_host_function("GetSkyColor", get_sky_color);
    script.register_host_function("SetGamma", set_gamma);
    script.register_host_function("ResetGamma", reset_gamma);
    script.register_host_function("GBackSolid", g_back_solid);
    script.register_host_function("GBackSemiSolid", g_back_semi_solid);
    script.register_host_function("GBackLiquid", g_back_liquid);
    script.register_host_function("GBackSky", g_back_sky);
    script.register_host_function("GetMaterial", get_material);
    script.register_host_function("GetTexture", get_texture);
    script.register_host_function("SetDir", set_dir);
    script.register_host_function("GetDir", get_dir);
    script.register_host_function("SetComDir", set_com_dir);
    script.register_host_function("GetComDir", get_com_dir);
    script.register_host_function("ExecuteCommand", execute_command);
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
    script.register_host_function("AdjustWalkRotation", adjust_walk_rotation);
    script.register_host_function("GetRDir", get_r_dir);
    script.register_host_function("FightWith", fight_with);
    script.register_host_function("FindBase", find_base);
    script.register_host_function("FindObject", find_object);
    script.register_host_function("FindObjectOwner", find_object_owner);
    script.register_host_function("FindObject2", find_object2);
    script.register_host_function("FindObjects", find_objects_dispatch);
    script.register_host_function("Find_AtPoint", find_at_point);
    script.register_host_function("Find_Category", find_category);
    script.register_host_function("Find_ID", find_id);
    script.register_host_function("ObjectNumber", object_number);
    script.register_host_function("Object", object_by_number);
    script.register_host_function("ObjectCount2", object_count2);
    script.register_host_function("ObjectCount", object_count);
    script.register_host_function("ObjectDistance", object_distance);
    script.register_host_function("GetX", get_x);
    script.register_host_function("GetY", get_y);
    script.register_host_function("GetDefBottom", get_def_bottom);
    script.register_host_function("GetID", get_id);
    script.register_host_function("GetBase", get_base);
    script.register_host_function("SetPosition", set_position);
    script.register_host_function("CreateObject", create_object);
    script.register_host_function("CastAny", cast_any);
    script.register_host_function("CastInt", cast_int);
    script.register_host_function("CastBool", cast_bool);
    script.register_host_function("CastC4ID", cast_c4id);
    script.register_host_function("CastObjects", cast_objects);
    script.register_host_function("CastPXS", cast_pxs);
    script.register_host_function("PlaceAnimal", place_animal);
    script.register_host_function("PlaceVegetation", place_vegetation);
    script.register_host_function("CreateConstruction", create_construction);
    // FnFindConstructionSite (C4Script.cpp:1958-1981) — the caller-Var
    // staging seam behind the System.c4g FindConstructionSiteX wrapper.
    script.register_host_function("FindConstructionSite", find_construction_site);
    script.register_host_function("CreateParticle", create_particle);
    script.register_host_function("CastParticles", cast_particles);
    script.register_host_function("CastBackParticles", cast_back_particles);
    script.register_host_function("PushParticles", push_particles);
    script.register_host_function("ClearParticles", clear_particles);
    script.register_host_function("IsNewgfx", is_newgfx);
    script.register_host_function("CustomMessage", custom_message);
    script.register_host_function("Message", message);
    script.register_host_function("PlayerMessage", player_message);
    script.register_host_function("AddMessage", add_message);
    script.register_host_function("PlrMessage", plr_message);
    script.register_host_function("Log", log_message);
    script.register_host_function("DebugLog", debug_log_message);
    script.register_host_function("FatalError", fatal_error);
    script.register_host_function("LocateFunc", locate_func);
    script.register_host_function("StartCallTrace", start_call_trace);
    script.register_host_function("StartScriptProfiler", start_script_profiler);
    script.register_host_function("StopScriptProfiler", stop_script_profiler);
    script.register_host_function("GameOver", game_over);
    script.register_host_function("GainMissionAccess", gain_mission_access);
    script.register_host_function("GetMissionAccess", get_mission_access);
    script.register_host_function("SetNextMission", set_next_mission);
    script.register_host_function("SetRestoreInfos", set_restore_infos);
    script.register_host_function("Call", call_self);
    script.register_host_function("ObjectCall", object_call);
    script.register_host_function("ProtectedCall", object_call);
    script.register_host_function("PrivateCall", object_call);
    script.register_host_function("DefinitionCall", definition_call);
    script.register_host_function("GameCall", game_call);
    script.register_host_function("GameCallEx", game_call_ex);
    script.register_host_function("Format", format_string);
    script.register_host_reference_function("Equal", std::iter::empty::<usize>(), equal);
    script.register_host_function("IsRef", is_ref);
    script.register_host_function("GetType", get_type);
    script.register_host_function("CreateArray", create_array);
    script.register_host_reference_function("Inc", [0], inc_reference);
    script.register_host_reference_function("Set", [0], set_reference);
    script.register_host_reference_function("Dec", [0], dec_reference);
    script.register_host_reference_function("SetLength", [0], set_length);
    script.register_host_function("GetLength", get_length);
    // Keep tracked argument provenance even though neither parameter is a
    // writable reference: NONSTRICT/STRICT1 compare C4Value backing pointers.
    script.register_host_reference_function(
        "GetIndexOf",
        std::iter::empty::<usize>(),
        get_index_of,
    );
    script.register_host_function("GetKeys", get_keys);
    script.register_host_function("GetValues", get_values);
    script.register_host_function("Contents", contents);
    script.register_host_function("ContentsCount", contents_count);
    script.register_host_function("FindContents", find_contents);
    script.register_host_function("FindOtherContents", find_other_contents);
    script.register_host_function("Contained", contained);
    script.register_host_function("GetCategory", get_category);
    script.register_host_function("SetCategory", set_category);
    script.register_host_function("NoContainer", no_container);
    script.register_host_function("AnyContainer", any_container);
    script.register_host_function("ActIdle", act_idle);
    script.register_host_function("CreateContents", create_contents);
    script.register_host_function("ComposeContents", compose_contents);
    script.register_host_function("Split2Components", split_to_components);
    script.register_host_function("GetActMapVal", get_act_map_val);
    script.register_host_function("GetObjectVal", get_object_val);
    script.register_host_function("GetObjectInfoCoreVal", get_object_info_core_val);
    // System.c4g/GetXVal.c:78-79 wrappers. The Rust loader does not yet
    // compile the engine-wide planet/System.c4g.
    script.register_host_function("GetObjWidth", get_obj_width);
    script.register_host_function("GetObjHeight", get_obj_height);
    script.register_host_function("GetEntrance", get_entrance);
    script.register_host_function("SetEntrance", set_entrance);
    script.register_host_function("SetColor", set_color);
    script.register_host_function("SetColorDw", set_color_dw);
    script.register_host_function("SetPicture", set_picture);
    script.register_host_function("SetShape", set_shape);
    script.register_host_function("AddVertex", add_vertex);
    script.register_host_function("RemoveVertex", remove_vertex);
    script.register_host_function("SetVertex", set_vertex);
    script.register_host_function("SetContactDensity", set_contact_density);
    script.register_host_function("SetAlive", set_alive);
    script.register_host_function("GetAlive", get_alive);
    script.register_host_function("SetOwner", set_owner);
    script.register_host_function("GetOwner", get_owner);
    script.register_host_function("CrewMember", crew_member);
    script.register_host_function("Distance", distance);
    script.register_host_function("SetViewOffset", set_view_offset);
    script.register_host_function("GetController", get_controller);
    script.register_host_function("SetController", set_controller);
    script.register_host_function("GetKiller", get_killer);
    script.register_host_function("SetKiller", set_killer);
    script.register_host_function("SetObjectStatus", set_object_status);
    script.register_host_function("GetObjectStatus", get_object_status);
    script.register_host_function("GetObjectLayer", get_object_layer);
    script.register_host_function("SetObjectLayer", set_object_layer);
    script.register_host_function("SetObjectOrder", set_object_order);
    script.register_host_function("Resort", resort);
    script.register_host_function("ResortObjects", resort_objects);
    script.register_host_function("ResortObject", resort_object);
    script.register_host_function("GetObjectBlitMode", get_object_blit_mode);
    script.register_host_function("SetObjectBlitMode", set_object_blit_mode);
    script.register_host_function("GetOCF", get_ocf);
    script.register_host_function("InsertMaterial", insert_material);
    script.register_host_function("ExtractLiquid", extract_liquid);
    script.register_host_function("FlameConsumeMaterial", flame_consume_material);
    script.register_host_function("ExtractMaterialAmount", extract_material_amount);
    script.register_host_function("IncinerateLandscape", incinerate_landscape);
    script.register_host_function("Incinerate", incinerate);
    script.register_host_function("Extinguish", extinguish);
    script.register_host_function("OnFire", on_fire);
    // The engine-internal fire effect callbacks (AddFunc,
    // C4Script.cpp:6994-6997) — script overloads chain back via
    // inherited(...).
    script.register_host_function("FxFireStart", fx_fire_start);
    script.register_host_function("FxFireTimer", fx_fire_timer);
    script.register_host_function("FxFireStop", fx_fire_stop);
    script.register_host_function("FxFireInfo", fx_fire_info);
    script.register_host_function("GetUnusedOverlayID", get_unused_overlay_id);
    script.register_host_function("SetGraphics", set_graphics);
    script.register_host_function("SetObjDrawTransform", set_obj_draw_transform);
    script.register_host_function("SetObjDrawTransform2", set_obj_draw_transform2);
    script.register_host_function("RemoveObject", remove_object);
    script.register_host_function("GetEnergy", get_energy);
    script.register_host_function("DoEnergy", do_energy);
    script.register_host_function("DeathAnnounce", death_announce);
    // FnDoMagicEnergy/FnGetMagicEnergy (C4Script.cpp:517-550, AddFunc
    // :6715-6716) — Fantasy's NoMagicEnergy.c4d global overrides chain to
    // these via inherited.
    script.register_host_function("DoMagicEnergy", do_magic_energy);
    script.register_host_function("DoScoreboardShow", do_scoreboard_show);
    script.register_host_function("GetMagicEnergy", get_magic_energy);
    script.register_host_function("GetPhysical", get_physical);
    script.register_host_function("SetPhysical", set_physical);
    script.register_host_function("TrainPhysical", train_physical);
    script.register_host_function("ResetPhysical", reset_physical);
    script.register_host_function("DoBreath", do_breath);
    script.register_host_function("GetBreath", get_breath);
    script.register_host_function("GetName", get_name);
    script.register_host_function("GetDesc", get_desc);
    script.register_host_function("SetName", set_name);
    script.register_host_function("GetCon", get_con);
    script.register_host_function("DoCon", do_con);
    script.register_host_function("DoDamage", do_damage);
    script.register_host_function("GetDamage", get_damage);
    script.register_host_function("GetPlrColorDw", get_plr_color_dw);
    script.register_host_function("GetPlrControlName", get_plr_control_name);
    script.register_host_function("GetPlrJumpAndRunControl", get_plr_jump_and_run_control);
    script.register_host_function("DoHomebaseMaterial", do_homebase_material);
    script.register_host_function("DoHomebaseProduction", do_homebase_production);
    script.register_host_function("AssignVar", set_var);
    script.register_host_function("SetVar", set_var);
    script.register_host_function("DecVar", dec_var);
    script.register_host_function("IncVar", inc_var);
    script.register_host_function("Not", legacy_not);
    script.register_host_function("Or", legacy_or);
    script.register_host_function("And", legacy_and);
    script.register_host_function("BitAnd", legacy_bit_and);
    script.register_host_function("Sum", legacy_sum);
    script.register_host_function("Sub", legacy_sub);
    script.register_host_function("Mul", legacy_mul);
    script.register_host_function("Div", legacy_div);
    script.register_host_function("LessThan", legacy_less_than);
    script.register_host_function("GreaterThan", legacy_greater_than);
    script.register_host_function("SEqual", legacy_s_equal);
    script.register_host_function("Random", random);
    script.register_host_function("AsyncRandom", async_random);
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
    script.register_host_function("ArcSin", arc_sin_func);
    script.register_host_function("ArcCos", arc_cos_func);
    script.register_host_function("Angle", angle_func);
    script.register_host_function("Mod", modulo);
    script.register_host_function("GetMass", get_mass);
    script.register_host_function("SetMass", set_mass);
    script.register_host_function("GrabObjectInfo", grab_object_info);
    script.register_host_function("MakeCrewMember", make_crew_member);
    script.register_host_function("C4Id", c4_id);
    script.register_host_function("ScoreboardCol", scoreboard_col);
    script.register_host_function("SortScoreboard", sort_scoreboard);
    script.register_host_function("AddEvaluationData", add_evaluation_data);
    script.register_host_function(
        "HideSettlementScoreInEvaluation",
        hide_settlement_score_in_evaluation,
    );
    script.register_host_function("Pow", pow_func);
    script.register_host_function("BoundBy", bound_by_func);
    script.register_host_function("Sin", sin_func);
    script.register_host_function("Cos", cos_func);
    script.register_host_function("SetTemperature", set_temperature);
    script.register_host_function("GetTemperature", get_temperature);
    script.register_host_function("SetClimate", set_climate);
    script.register_host_function("GetClimate", get_climate);
    script.register_host_function("SetSeason", set_season);
    script.register_host_function("GetSeason", get_season);
    script.register_host_function("Music", music);
    script.register_host_function("MusicLevel", music_level);
    script.register_host_function("SetPlayList", set_play_list);
    script.register_host_function("Sound", sound);
    script.register_host_function("SoundLevel", sound_level);

    // CheckConvertFunctionParameters runs in the VM before debugger hooks.
    // Wrap only ordinary C++ AddFunc callbacks here so their bodies receive
    // the later native primitive extraction performed by C4AulEngineFunc.
    install_cpp_add_func_argument_extractors(script.engine);

    // C4AulEngineFunc retains `C4ValueConv<Par>::Type()` for every native
    // slot. Keep this separate declarative table authoritative for both the
    // boundary conversion and exact arity of the Rust registrations above.
    for (name, parameter_types) in
        crate::native_function_parameters::native_function_parameter_entries()
    {
        assert!(
            script
                .engine
                .set_host_function_parameter_types(name, parameter_types.iter().copied()),
            "native signature exists without a registered callback: {name}"
        );
    }
}

fn install_host_dispatch_hooks(script: &mut ScriptEngine) {
    script.register_method_dispatch(std::sync::Arc::new(arrow_method_dispatch));
    script.register_method_reference_dispatch(std::rc::Rc::new(arrow_method_reference_dispatch));
    script.register_global_call_context_hook(std::sync::Arc::new(global_call_context_hook));
    script.register_local_cell_hook(std::rc::Rc::new(foreign_local_cell_hook));
}

pub fn register_host_functions(script: &mut ScriptEngine) {
    static REGISTRATIONS: std::sync::OnceLock<HostRegistrationSnapshot> =
        std::sync::OnceLock::new();
    let registrations = REGISTRATIONS.get_or_init(|| {
        let mut template = ScriptEngine::new();
        populate_host_registration_template(&mut template);
        template.host_registration_snapshot()
    });
    script.apply_host_registration_snapshot(registrations);
    install_host_dispatch_hooks(script);
}

/// Native call frames include every declared slot, so the Rust shorthand
/// parsers below may leave only VM-padded nil values after consuming their
/// compact argument form. Those slots are indistinguishable from omitted
/// parameters in C++ and must not be diagnosed as surplus arguments.
pub(crate) fn has_remaining_native_argument(args: &[Value], start: usize) -> bool {
    args.get(start..)
        .is_some_and(|tail| tail.iter().any(|value| !matches!(value, Value::Nil)))
}

/// `C4AUL_MAX_Par` (C4Aul.h:54): the NumVars/Par slot count that bounds
/// FindConstructionSite's var indices.
pub(crate) const AUL_MAX_PAR: i32 = 10;
