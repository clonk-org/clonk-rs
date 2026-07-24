//! Declared C4Value parameter types for the production compatibility natives.
//!
//! The ordinary entries are transcribed from `AddFunc` in
//! `src/C4Script.cpp`: `C4AulEngineFunc` derives each slot through
//! `C4ValueConv<Par>::Type()`. The cast and ten-slot search functions use
//! the explicit `C4AulDefCastFunc` / `C4AulEngineFuncParArray`
//! registrations at the end of `InitFunctionMap`.
//!
//! Seven Rust registrations stand in for script conveniences rather than an
//! `AddFunc` entry. `Find_AtPoint`, `Find_Category`, and `Find_ID`
//! retain the typed declarations in `System.c4g/FindObject.c`;
//! `GetObjWidth` and `GetObjHeight` retain the object declaration in
//! `System.c4g/GetXVal.c`. `GetActionData` is an object-defaulting sibling
//! of the typed action getters. `GetVertexContact` follows its Rust callback's
//! declared positional contract: vertex index, check mask, then object.

use clonk_script::C4VType;
use clonk_script::C4VType::{Any, Array, Bool, C4Id, C4Object, Int, Map, Ref, String};

pub(crate) type NativeFunctionParameterEntry = (&'static str, &'static [C4VType]);

pub(crate) const CPP_BACKED_NATIVE_FUNCTION_COUNT: usize = 450;
pub(crate) const RUST_STANDIN_NATIVE_FUNCTIONS: &[&str] = &[
    "Find_AtPoint",
    "Find_Category",
    "Find_ID",
    "GetActionData",
    "GetObjHeight",
    "GetObjWidth",
    "GetVertexContact",
];

/// Sorted by function name so lookup is deterministic and logarithmic.
pub(crate) const NATIVE_FUNCTION_PARAMETERS: &[NativeFunctionParameterEntry] = &[
    ("AbortMessageBoard", &[C4Object, Int]),
    ("Abs", &[Int]),
    ("ActIdle", &[C4Object]),
    ("ActivateGameGoalMenu", &[Int]),
    (
        "AddCommand",
        &[
            C4Object, String, C4Object, Any, Int, C4Object, Int, Any, Int, Int,
        ],
    ),
    (
        "AddEffect",
        &[
            String, C4Object, Int, Int, C4Object, C4Id, Any, Any, Any, Any,
        ],
    ),
    ("AddEvaluationData", &[String, Int]),
    (
        "AddMenuItem",
        &[
            String, String, C4Id, C4Object, Int, Any, String, Int, Any, Any,
        ],
    ),
    (
        "AddMessage",
        &[String, C4Object, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("AddMsgBoardCmd", &[String, String, Int]),
    ("AddVertex", &[Int, Int, C4Object]),
    ("AdjustWalkRotation", &[Int, Int, Int, C4Object]),
    ("And", &[Bool, Bool]),
    ("Angle", &[Int, Int, Int, Int, Int]),
    ("AnyContainer", &[]),
    (
        "AppendCommand",
        &[
            C4Object, String, C4Object, Any, Int, C4Object, Int, Any, Int, Int,
        ],
    ),
    ("ArcCos", &[Int, Int]),
    ("ArcSin", &[Int, Int]),
    ("AssignVar", &[Int, Any]),
    ("AsyncRandom", &[Int]),
    ("BitAnd", &[Int, Int]),
    ("BlastFree", &[Int, Int, Int, Int]),
    ("BlastObject", &[Int, C4Object, Int]),
    ("BlastObjects", &[Int, Int, Int, C4Object, Int]),
    ("BoundBy", &[Int, Int, Int]),
    ("Bubble", &[Int, Int]),
    ("Buy", &[C4Id, Int, Int, C4Object, Bool]),
    ("C4Id", &[String]),
    (
        "Call",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("CallMessageBoard", &[C4Object, Bool, String, Int]),
    // C4AulDefCastFunc registrations declare their source type, not the cast result.
    ("CastAny", &[Any]),
    (
        "CastBackParticles",
        &[String, Int, Int, Int, Int, Int, Int, Int, Int, C4Object],
    ),
    ("CastBool", &[Any]),
    ("CastC4ID", &[Any]),
    ("CastInt", &[Any]),
    ("CastObjects", &[C4Id, Int, Int, Int, Int]),
    ("CastPXS", &[String, Int, Int, Int, Int]),
    (
        "CastParticles",
        &[String, Int, Int, Int, Int, Int, Int, Int, Int, C4Object],
    ),
    ("ChangeDef", &[C4Id, C4Object]),
    ("ChangeEffect", &[String, C4Object, Int, String, Int]),
    (
        "CheckEffect",
        &[String, C4Object, Int, Int, Any, Any, Any, Any],
    ),
    ("CheckEnergyNeedChain", &[C4Object]),
    ("ClearLastPlrCom", &[Int]),
    ("ClearMenuItems", &[C4Object]),
    ("ClearParticles", &[String, C4Object]),
    ("CloseMenu", &[C4Object]),
    ("Collect", &[C4Object, C4Object]),
    ("ComponentAll", &[C4Object, C4Id]),
    ("ComposeContents", &[C4Id, C4Object]),
    ("Contained", &[C4Object]),
    ("Contents", &[Int, C4Object, Bool]),
    ("ContentsCount", &[C4Id, C4Object]),
    ("Cos", &[Int, Int, Int]),
    ("CreateArray", &[Int]),
    (
        "CreateConstruction",
        &[C4Id, Int, Int, Int, Int, Bool, Bool],
    ),
    ("CreateContents", &[C4Id, C4Object, Int]),
    (
        "CreateMenu",
        &[C4Id, C4Object, C4Object, Int, String, Int, Int, Bool, C4Id],
    ),
    ("CreateObject", &[C4Id, Int, Int, Int]),
    (
        "CreateParticle",
        &[String, Int, Int, Int, Int, Int, Int, C4Object, Bool],
    ),
    ("CreateScriptPlayer", &[String, Int, Int, Int, C4Id]),
    ("CrewMember", &[C4Object]),
    (
        "CustomMessage",
        &[String, C4Object, Int, Int, Int, Int, C4Id, String, Int, Int],
    ),
    ("DeathAnnounce", &[]),
    (
        "DebugLog",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("Dec", &[Ref, Any]),
    ("DecVar", &[Int]),
    (
        "DefinitionCall",
        &[C4Id, String, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("DigFree", &[Int, Int, Int, Bool]),
    ("DigFreeRect", &[Int, Int, Int, Int, Bool]),
    ("Distance", &[Int, Int, Int, Int]),
    ("Div", &[Int, Int]),
    ("DoBreath", &[Int, C4Object]),
    ("DoCon", &[Int, C4Object]),
    ("DoCrewExp", &[Int, C4Object]),
    ("DoDamage", &[Int, C4Object, Int, Int]),
    ("DoEnergy", &[Int, C4Object, Bool, Int, Int]),
    ("DoHomebaseMaterial", &[Int, C4Id, Int]),
    ("DoHomebaseProduction", &[Int, C4Id, Int]),
    ("DoMagicEnergy", &[Int, C4Object, Bool]),
    ("DoScore", &[Int, Int]),
    ("DoScoreboardShow", &[Int, Int]),
    ("DrawDefMap", &[Int, Int, Int, Int, String]),
    ("DrawMap", &[Int, Int, Int, Int, String]),
    (
        "DrawMatChunks",
        &[Int, Int, Int, Int, Int, Int, String, String, Bool],
    ),
    (
        "DrawMaterialQuad",
        &[String, Int, Int, Int, Int, Int, Int, Int, Int, Bool],
    ),
    ("DrawVolcanoBranch", &[Int, Int, Int, Int, Int, Int]),
    ("EditCursor", &[]),
    (
        "EffectCall",
        &[C4Object, Int, String, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("EffectVar", &[Int, C4Object, Int]),
    ("EliminatePlayer", &[Int, Bool]),
    ("EnergyCheck", &[Int, C4Object]),
    ("Enter", &[C4Object, C4Object]),
    ("Equal", &[Any, Any]),
    ("ExecuteCommand", &[C4Object]),
    ("Exit", &[C4Object, Int, Int, Int, Int, Int, Int]),
    ("Explode", &[Int, C4Object, C4Id, String]),
    ("Extinguish", &[C4Object]),
    ("ExtractLiquid", &[Int, Int]),
    ("ExtractMaterialAmount", &[Int, Int, Int, Int]),
    ("FatalError", &[String]),
    ("FightWith", &[C4Object, C4Object]),
    ("FindBase", &[Int, Int]),
    ("FindConstructionSite", &[C4Id, Int, Int]),
    ("FindContents", &[C4Id, C4Object]),
    (
        "FindObject",
        &[
            C4Id, Int, Int, Int, Int, Int, String, C4Object, Any, C4Object,
        ],
    ),
    // C4AulEngineFuncParArray registrations: all ten slots are declared explicitly.
    (
        "FindObject2",
        &[
            Array, Array, Array, Array, Array, Array, Array, Array, Array, Array,
        ],
    ),
    (
        "FindObjectOwner",
        &[
            C4Id, Int, Int, Int, Int, Int, Int, String, C4Object, C4Object,
        ],
    ),
    (
        "FindObjects",
        &[Array, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("FindOtherContents", &[C4Id, C4Object]),
    // Rust stand-ins for typed System.c4g wrappers and two legacy convenience callbacks.
    // These first five callbacks replace typed System.c4g script functions.
    // C4AulScriptFunc keeps the base ten-slot GetParCount and initializes
    // every undeclared ParType entry to Any (C4Aul.h:305,337-353).
    (
        "Find_AtPoint",
        &[Int, Int, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "Find_Category",
        &[Int, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "Find_ID",
        &[C4Id, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("FinishCommand", &[C4Object, Bool, Int]),
    ("FlameConsumeMaterial", &[Int, Int]),
    ("Fling", &[C4Object, Int, Int, Int, Bool]),
    (
        "Format",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("FrameCounter", &[]),
    ("FreeRect", &[Int, Int, Int, Int, Int]),
    ("FxFireInfo", &[C4Object, Int]),
    ("FxFireStart", &[C4Object, Int, Int, Int, Bool, C4Object]),
    ("FxFireStop", &[C4Object, Int, Int, Bool]),
    ("FxFireTimer", &[C4Object, Int, Int]),
    ("GBackLiquid", &[Int, Int]),
    ("GBackSemiSolid", &[Int, Int]),
    ("GBackSky", &[Int, Int]),
    ("GBackSolid", &[Int, Int]),
    ("GainMissionAccess", &[String]),
    (
        "GameCall",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "GameCallEx",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("GameOver", &[Int]),
    ("GetActMapVal", &[String, String, C4Id, Int]),
    ("GetActTime", &[C4Object]),
    ("GetAction", &[C4Object]),
    ("GetActionData", &[C4Object]),
    ("GetActionTarget", &[Int, C4Object]),
    ("GetAlive", &[C4Object]),
    ("GetBase", &[C4Object]),
    ("GetBreath", &[C4Object]),
    ("GetCaptain", &[Int]),
    ("GetCategory", &[C4Object, C4Id]),
    ("GetChar", &[String, Int]),
    ("GetClimate", &[]),
    ("GetClrModulation", &[C4Object, Int]),
    ("GetColor", &[C4Object]),
    ("GetColorDw", &[C4Object]),
    ("GetComDir", &[C4Object]),
    ("GetCommand", &[C4Object, Int, Int]),
    ("GetComponent", &[C4Id, Int, C4Object, C4Id]),
    ("GetCon", &[C4Object]),
    ("GetContact", &[C4Object, Int, Int]),
    ("GetController", &[C4Object]),
    ("GetCrew", &[Int, Int]),
    ("GetCrewCount", &[Int]),
    ("GetCrewEnabled", &[C4Object]),
    ("GetCrewExtraData", &[C4Object, String]),
    ("GetCursor", &[Int, Int]),
    ("GetDamage", &[C4Object]),
    ("GetDefBottom", &[C4Object]),
    ("GetDefCoreVal", &[String, String, C4Id, Int]),
    ("GetDefinition", &[Int, Int]),
    ("GetDesc", &[C4Object, C4Id]),
    ("GetDir", &[C4Object]),
    ("GetEffect", &[String, C4Object, Int, Int, Int]),
    ("GetEffectCount", &[String, C4Object, Int]),
    ("GetEnergy", &[C4Object]),
    ("GetEntrance", &[C4Object]),
    ("GetGravity", &[]),
    ("GetHiRank", &[Int]),
    ("GetHomebaseMaterial", &[Int, C4Id, Int, Int]),
    ("GetHomebaseProduction", &[Int, C4Id, Int, Int]),
    ("GetID", &[C4Object]),
    ("GetIndexOf", &[Any, Array]),
    ("GetKeys", &[Map]),
    ("GetKiller", &[C4Object]),
    ("GetLeague", &[Int]),
    ("GetLeagueProgressData", &[Int]),
    ("GetLeagueScore", &[Int]),
    ("GetLength", &[Any]),
    ("GetMagicEnergy", &[C4Object]),
    ("GetMass", &[C4Object, C4Id]),
    ("GetMatAdjust", &[]),
    ("GetMaterial", &[Int, Int]),
    ("GetMaterialColor", &[Int, Int, Int]),
    ("GetMaterialCount", &[Int, Bool]),
    ("GetMaterialVal", &[String, String, Int, Int]),
    ("GetMaxPlayer", &[]),
    ("GetMenu", &[C4Object]),
    ("GetMenuSelection", &[C4Object]),
    ("GetMissionAccess", &[String]),
    ("GetName", &[C4Object, C4Id]),
    ("GetNeededMatStr", &[C4Object]),
    ("GetOCF", &[C4Object]),
    (
        "GetObjHeight",
        &[C4Object, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "GetObjWidth",
        &[C4Object, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("GetObjectBlitMode", &[C4Object, Int]),
    ("GetObjectInfoCoreVal", &[String, String, C4Object, Int]),
    ("GetObjectLayer", &[C4Object]),
    ("GetObjectStatus", &[C4Object]),
    ("GetObjectVal", &[String, String, C4Object, Int]),
    ("GetOwner", &[C4Object]),
    ("GetPath", &[Int, Int, Int, Int]),
    ("GetPhase", &[C4Object]),
    ("GetPhysical", &[String, Int, C4Object, C4Id]),
    ("GetPlayerByIndex", &[Int, Int]),
    ("GetPlayerCount", &[Int]),
    ("GetPlayerID", &[Int]),
    ("GetPlayerInfoCoreVal", &[String, String, Int, Int]),
    ("GetPlayerName", &[Int]),
    ("GetPlayerTeam", &[Int]),
    ("GetPlayerType", &[Int]),
    ("GetPlayerVal", &[String, String, Int, Int]),
    ("GetPlrColorDw", &[Int]),
    ("GetPlrControlName", &[Int, Int, Bool]),
    ("GetPlrDownDouble", &[Int]),
    ("GetPlrExtraData", &[Int, String]),
    ("GetPlrJumpAndRunControl", &[Int]),
    ("GetPlrKnowledge", &[Int, C4Id, Int, Int]),
    ("GetPlrMagic", &[Int, C4Id, Int]),
    ("GetPlrValue", &[Int]),
    ("GetPlrValueGain", &[Int]),
    ("GetPlrView", &[Int]),
    ("GetPlrViewMode", &[Int]),
    ("GetPortrait", &[C4Object, Bool, Bool]),
    ("GetProcedure", &[C4Object]),
    ("GetR", &[C4Object]),
    ("GetRDir", &[C4Object, Int]),
    ("GetRank", &[C4Object]),
    ("GetScenarioVal", &[String, String, Int]),
    ("GetScore", &[Int]),
    ("GetScoreboardData", &[Int, Int]),
    ("GetScoreboardString", &[Int, Int]),
    ("GetSeason", &[]),
    ("GetSelectCount", &[Int]),
    ("GetSkyAdjust", &[Bool]),
    ("GetSkyColor", &[Int, Int]),
    ("GetSystemTime", &[Int]),
    ("GetTaggedPlayerName", &[Int]),
    ("GetTeamByIndex", &[Int]),
    ("GetTeamColor", &[Int]),
    ("GetTeamConfig", &[Int]),
    ("GetTeamCount", &[]),
    ("GetTeamName", &[Int]),
    ("GetTemperature", &[]),
    ("GetTexture", &[Int, Int]),
    ("GetTime", &[]),
    ("GetType", &[Any]),
    ("GetUnusedOverlayID", &[Int, C4Object]),
    ("GetValue", &[C4Object, C4Id, C4Object, Int]),
    ("GetValues", &[Map]),
    ("GetVertex", &[Int, Int, C4Object]),
    ("GetVertexContact", &[Int, Int, C4Object]),
    ("GetVertexNum", &[C4Object]),
    ("GetViewCursor", &[Int]),
    ("GetVisibility", &[C4Object]),
    ("GetWealth", &[Int]),
    ("GetWind", &[Int, Int, Bool]),
    ("GetX", &[C4Object]),
    ("GetXDir", &[C4Object, Int]),
    ("GetY", &[C4Object]),
    ("GetYDir", &[C4Object, Int]),
    ("GrabContents", &[C4Object, C4Object]),
    ("GrabObjectInfo", &[C4Object, C4Object]),
    ("GreaterThan", &[Int, Int]),
    ("HideSettlementScoreInEvaluation", &[Bool]),
    ("Hostile", &[Int, Int, Bool]),
    ("InLiquid", &[C4Object]),
    ("Inc", &[Ref, Any]),
    ("IncVar", &[Int]),
    ("Incinerate", &[C4Object]),
    ("IncinerateLandscape", &[Int, Int]),
    ("InitScenarioPlayer", &[Int, Int]),
    ("InsertMaterial", &[Int, Int, Int, Int, Int]),
    ("Inside", &[Int, Int, Int]),
    ("IsNetwork", &[]),
    ("IsNewgfx", &[]),
    ("IsRef", &[Any]),
    ("Jump", &[C4Object]),
    ("Kill", &[C4Object, Bool]),
    ("LandscapeHeight", &[]),
    ("LandscapeWidth", &[]),
    ("LaunchEarthquake", &[Int, Int]),
    ("LaunchLightning", &[Int, Int, Int, Int, Int, Int, Bool]),
    ("LaunchVolcano", &[Int]),
    ("LessThan", &[Int, Int]),
    ("LoadScenarioSection", &[String, Int]),
    ("LocateFunc", &[String, C4Object, C4Id]),
    (
        "Log",
        &[String, Any, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("MakeCrewMember", &[C4Object, Int]),
    ("Material", &[String]),
    ("MaterialName", &[Int]),
    ("Max", &[Int, Int]),
    (
        "Message",
        &[String, C4Object, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("Min", &[Int, Int]),
    ("Mod", &[Int, Int]),
    ("ModulateColor", &[Int, Int]),
    ("Mul", &[Int, Int]),
    ("Music", &[String, Bool]),
    ("MusicLevel", &[Int]),
    ("NoContainer", &[]),
    ("Not", &[Bool]),
    ("Object", &[Int]),
    (
        "ObjectCall",
        &[C4Object, String, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "ObjectCount",
        &[C4Id, Int, Int, Int, Int, Int, String, C4Object, Any, Int],
    ),
    (
        "ObjectCount2",
        &[
            Array, Array, Array, Array, Array, Array, Array, Array, Array, Array,
        ],
    ),
    ("ObjectDistance", &[C4Object, C4Object]),
    ("ObjectNumber", &[C4Object]),
    (
        "ObjectSetAction",
        &[C4Object, String, C4Object, C4Object, Bool],
    ),
    ("OnFire", &[C4Object]),
    ("OnMessageBoardAnswer", &[C4Object, Int, String]),
    ("Or", &[Bool, Bool, Bool, Bool, Bool]),
    ("PathFree", &[Int, Int, Int, Int]),
    ("PathFree2", &[Ref, Ref, Int, Int]),
    ("PauseGame", &[Bool]),
    ("PlaceAnimal", &[C4Id]),
    ("PlaceVegetation", &[C4Id, Int, Int, Int, Int, Int]),
    (
        "PlayerMessage",
        &[Int, String, C4Object, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "PlayerObjectCommand",
        &[Int, String, C4Object, Any, Int, C4Object, Any],
    ),
    (
        "PlrMessage",
        &[String, Int, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("Pow", &[Int, Int]),
    (
        "PrivateCall",
        &[C4Object, String, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    (
        "ProtectedCall",
        &[C4Object, String, Any, Any, Any, Any, Any, Any, Any, Any],
    ),
    ("Punch", &[C4Object, Int]),
    ("PushParticles", &[String, Int, Int]),
    ("Random", &[Int]),
    ("ReloadDef", &[C4Id]),
    ("ReloadParticle", &[String]),
    ("RemoveEffect", &[String, C4Object, Int, Bool]),
    ("RemoveObject", &[C4Object, Bool]),
    ("RemoveUnusedTexMapEntries", &[]),
    ("RemoveVertex", &[Int, C4Object]),
    ("ResetGamma", &[Int]),
    ("ResetPhysical", &[C4Object, String]),
    ("Resort", &[C4Object]),
    ("ResortObject", &[String, C4Object]),
    ("ResortObjects", &[String, Int]),
    ("SEqual", &[String, String]),
    // ScoreboardCol is the remaining C4AulDefCastFunc; its source is C4V_C4ID.
    ("ScoreboardCol", &[C4Id]),
    ("ScriptCounter", &[]),
    ("ScriptGo", &[Bool]),
    ("ScrollContents", &[C4Object]),
    ("SelectCrew", &[Int, C4Object, Bool, Bool]),
    ("SelectMenuItem", &[Int, C4Object]),
    ("Sell", &[Int, C4Object]),
    ("Set", &[Ref, Any]),
    ("SetAction", &[String, C4Object, C4Object, Bool]),
    ("SetActionData", &[Int, C4Object]),
    ("SetActionTargets", &[C4Object, C4Object, C4Object]),
    ("SetAlive", &[Bool, C4Object]),
    ("SetBridgeActionData", &[Int, Bool, Bool, Int, C4Object]),
    ("SetCategory", &[Int, C4Object]),
    ("SetClimate", &[Int]),
    ("SetClrModulation", &[Int, C4Object, Int]),
    ("SetColor", &[Int, C4Object]),
    ("SetColorDw", &[Int, C4Object]),
    ("SetComDir", &[Int, C4Object]),
    (
        "SetCommand",
        &[C4Object, String, C4Object, Any, Int, C4Object, Any, Int],
    ),
    ("SetComponent", &[C4Id, Int, C4Object]),
    ("SetContactDensity", &[Int, C4Object]),
    ("SetController", &[Int, C4Object]),
    ("SetCrewEnabled", &[Bool, C4Object]),
    ("SetCrewExtraData", &[C4Object, String, Any]),
    ("SetCrewStatus", &[Int, Bool, C4Object]),
    ("SetCursor", &[Int, C4Object, Bool, Bool, Bool]),
    ("SetDir", &[Int, C4Object]),
    ("SetEntrance", &[Bool, C4Object]),
    ("SetFilmView", &[Int]),
    ("SetFoW", &[Bool, Int]),
    ("SetGameSpeed", &[Int]),
    ("SetGamma", &[Int, Int, Int, Int]),
    (
        "SetGraphics",
        &[String, C4Object, C4Id, Int, Int, String, Int, C4Object],
    ),
    ("SetGravity", &[Int]),
    ("SetHostility", &[Int, Int, Bool, Bool, Bool]),
    ("SetKiller", &[Int, C4Object]),
    ("SetLandscapePixel", &[Int, Int, Int]),
    ("SetLeaguePerformance", &[Int, Int]),
    ("SetLeagueProgressData", &[String, Int]),
    ("SetLength", &[Ref, Int]),
    ("SetMass", &[Int, C4Object]),
    ("SetMatAdjust", &[Int]),
    (
        "SetMaterialColor",
        &[Int, Int, Int, Int, Int, Int, Int, Int, Int, Int],
    ),
    ("SetMaxPlayer", &[Int]),
    ("SetMenuDecoration", &[C4Id, C4Object]),
    ("SetMenuSize", &[Int, Int, C4Object]),
    ("SetMenuTextProgress", &[Int, C4Object]),
    ("SetName", &[String, C4Object, C4Id, Bool, Bool]),
    ("SetNextMission", &[String, String, String]),
    (
        "SetObjDrawTransform",
        &[Int, Int, Int, Int, Int, Int, C4Object, Int],
    ),
    (
        "SetObjDrawTransform2",
        &[Int, Int, Int, Int, Int, Int, Int, Int, Int, Int],
    ),
    ("SetObjectBlitMode", &[Int, C4Object, Int]),
    ("SetObjectLayer", &[C4Object, C4Object]),
    ("SetObjectOrder", &[C4Object, C4Object, Bool]),
    ("SetObjectStatus", &[Int, C4Object, Bool]),
    ("SetOwner", &[Int, C4Object]),
    ("SetPhase", &[Int, C4Object]),
    ("SetPhysical", &[String, Int, Int, C4Object]),
    ("SetPicture", &[Int, Int, Int, Int, C4Object]),
    ("SetPlayList", &[String, Bool]),
    ("SetPlayerTeam", &[Int, Int, Bool]),
    ("SetPlrExtraData", &[Int, String, Any]),
    ("SetPlrKnowledge", &[Int, C4Id, Bool]),
    ("SetPlrMagic", &[Int, C4Id, Bool]),
    ("SetPlrShowCommand", &[Int, Int]),
    ("SetPlrShowControl", &[Int, String]),
    ("SetPlrShowControlPos", &[Int, Int]),
    ("SetPlrView", &[Int, C4Object]),
    ("SetPlrViewRange", &[Int, C4Object, Bool]),
    ("SetPortrait", &[String, C4Object, C4Id, Bool, Bool]),
    ("SetPosition", &[Int, Int, C4Object, Bool]),
    ("SetPreSend", &[Int, String]),
    ("SetR", &[Int, C4Object]),
    ("SetRDir", &[Int, C4Object, Int]),
    ("SetRestoreInfos", &[Int]),
    ("SetScoreboardData", &[Int, Int, String, Int]),
    ("SetSeason", &[Int]),
    ("SetShape", &[Int, Int, Int, Int, C4Object]),
    ("SetSkyAdjust", &[Int, Int]),
    ("SetSkyColor", &[Int, Int, Int, Int]),
    ("SetSkyFade", &[Int, Int, Int, Int, Int, Int]),
    ("SetSkyParallax", &[Int, Int, Int, Int, Int, Int, Int]),
    ("SetSolidMask", &[Int, Int, Int, Int, Int, Int, C4Object]),
    ("SetTemperature", &[Int]),
    ("SetTextureIndex", &[String, Int, Bool]),
    ("SetTransferZone", &[Int, Int, Int, Int, C4Object]),
    ("SetVar", &[Int, Any]),
    ("SetVertex", &[Int, Int, Int, C4Object, Int]),
    ("SetViewCursor", &[Int, C4Object]),
    ("SetViewOffset", &[Int, Int, Int]),
    ("SetVisibility", &[Int, C4Object]),
    ("SetWealth", &[Int, Int]),
    ("SetWind", &[Int]),
    ("SetXDir", &[Int, C4Object, Int]),
    ("SetYDir", &[Int, C4Object, Int]),
    ("ShakeFree", &[Int, Int, Int]),
    ("ShakeObjects", &[Int, Int, Int]),
    ("ShiftContents", &[C4Object, Bool, C4Id, Bool]),
    ("ShowInfo", &[C4Object]),
    ("SimFlight", &[Ref, Ref, Ref, Ref, Int, Int, Int, Int]),
    ("Sin", &[Int, Int, Int]),
    ("Smoke", &[Int, Int, Int, Int]),
    ("SortScoreboard", &[Int, Bool]),
    ("Sound", &[String, Bool, C4Object, Int, Int, Int, Bool, Int]),
    ("SoundLevel", &[String, Int, C4Object]),
    ("Split2Components", &[C4Object]),
    ("Sqrt", &[Int]),
    ("StartCallTrace", &[]),
    ("StartScriptProfiler", &[C4Id]),
    ("StopScriptProfiler", &[]),
    ("Stuck", &[C4Object]),
    ("Sub", &[Int, Int, Int, Int]),
    ("Sum", &[Int, Int, Int, Int]),
    ("SurrenderPlayer", &[Int]),
    ("TestMessageBoard", &[Int, Bool]),
    ("TrainPhysical", &[String, Int, Int, C4Object]),
    ("UnselectCrew", &[Int]),
    ("Value", &[C4Id]),
    ("WildcardMatch", &[String, String]),
    ("goto", &[Int]),
];

pub(crate) fn native_function_parameters(name: &str) -> Option<&'static [C4VType]> {
    NATIVE_FUNCTION_PARAMETERS
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| NATIVE_FUNCTION_PARAMETERS[index].1)
}

pub(crate) fn native_function_parameter_entries(
) -> impl ExactSizeIterator<Item = NativeFunctionParameterEntry> {
    NATIVE_FUNCTION_PARAMETERS.iter().copied()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use clonk_script::C4VType::{Any, Array, C4Id, C4Object, Int, Ref};

    use super::{
        native_function_parameter_entries, native_function_parameters,
        CPP_BACKED_NATIVE_FUNCTION_COUNT, NATIVE_FUNCTION_PARAMETERS,
        RUST_STANDIN_NATIVE_FUNCTIONS,
    };

    fn production_registration_names() -> Vec<&'static str> {
        // `populate_host_registration_template` lives in the `registration`
        // compat submodule; brace-match its own body instead of an
        // incidentally-nearby marker, since family regrouping no longer
        // guarantees any particular item follows it in file order.
        let source = include_str!("compat/registration.rs");
        let marker = "fn populate_host_registration_template(script: &mut ScriptEngine) {";
        let start = source
            .find(marker)
            .expect("production registration function exists");
        let body_start = start + marker.len();
        let mut depth = 1i32;
        let mut body_end = body_start;
        for (offset, ch) in source[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = body_start + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            depth == 0,
            "production registration function end brace exists"
        );
        let registrations = &source[body_start..body_end];

        registrations
            .split("script.register_host_")
            .skip(1)
            .map(|registration| {
                let (_, after_quote) = registration
                    .split_once('"')
                    .expect("registration has a literal name");
                after_quote
                    .split_once('"')
                    .expect("registration name has a closing quote")
                    .0
            })
            .collect()
    }

    #[test]
    fn every_production_registration_has_exactly_one_signature() {
        let registrations = production_registration_names();
        assert_eq!(registrations.len(), 457);

        let mut registration_counts = BTreeMap::new();
        for name in registrations {
            *registration_counts.entry(name).or_insert(0usize) += 1;
        }
        assert!(
            registration_counts.values().all(|count| *count == 1),
            "production host names must be registered exactly once"
        );

        let mut signature_counts = BTreeMap::new();
        for (name, _) in native_function_parameter_entries() {
            *signature_counts.entry(name).or_insert(0usize) += 1;
        }
        assert_eq!(signature_counts.len(), 457);
        assert!(
            signature_counts.values().all(|count| *count == 1),
            "native host names must have exactly one signature"
        );
        assert_eq!(
            registration_counts.keys().copied().collect::<BTreeSet<_>>(),
            signature_counts.keys().copied().collect::<BTreeSet<_>>()
        );
        assert_eq!(
            CPP_BACKED_NATIVE_FUNCTION_COUNT + RUST_STANDIN_NATIVE_FUNCTIONS.len(),
            NATIVE_FUNCTION_PARAMETERS.len()
        );
    }

    #[test]
    fn special_registration_vectors_match_cpp() {
        assert_eq!(
            native_function_parameters("ScoreboardCol"),
            Some(&[C4Id][..])
        );
        for name in ["CastAny", "CastBool", "CastC4ID", "CastInt"] {
            assert_eq!(native_function_parameters(name), Some(&[Any][..]));
        }
        assert_eq!(
            native_function_parameters("FindObject2"),
            Some(&[Array; 10][..])
        );
        assert_eq!(
            native_function_parameters("FindObjects"),
            Some(&[Array, Any, Any, Any, Any, Any, Any, Any, Any, Any][..])
        );
        assert_eq!(
            native_function_parameters("ObjectCount2"),
            Some(&[Array; 10][..])
        );
        assert_eq!(
            native_function_parameters("SimFlight"),
            Some(&[Ref, Ref, Ref, Ref, Int, Int, Int, Int][..])
        );
        assert_eq!(
            native_function_parameters("GetVertexContact"),
            Some(&[Int, Int, C4Object][..])
        );
        assert_eq!(
            native_function_parameters("Find_AtPoint"),
            Some(&[Int, Int, Any, Any, Any, Any, Any, Any, Any, Any][..])
        );
        for (name, first) in [
            ("Find_Category", Int),
            ("Find_ID", C4Id),
            ("GetObjHeight", C4Object),
            ("GetObjWidth", C4Object),
        ] {
            let mut expected = [Any; 10];
            expected[0] = first;
            assert_eq!(native_function_parameters(name), Some(&expected[..]));
        }
    }

    #[test]
    fn registry_slot_count_matches_the_audited_cpp_surface() {
        assert_eq!(
            NATIVE_FUNCTION_PARAMETERS
                .iter()
                .map(|(_, parameters)| parameters.len())
                .sum::<usize>(),
            1_340
        );
    }
}
