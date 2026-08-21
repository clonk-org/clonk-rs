// LegacyClonk C++<->Rust differential parity oracle.
//
// This program is the GOLDEN ORACLE generator for the Phase-1 differential
// harness. It exercises the determinism-critical C++ primitives that the Rust
// port (crates/clonk-engine) must reproduce bit-for-bit, and emits the
// results as JSON to stdout. The Rust side (crates/clonk-engine/src/parity_differential.rs)
// runs the identical inputs and asserts byte-exact equality against the
// committed golden (parity/golden/parity_golden.json).
//
// The oracle deliberately uses the REAL engine code:
//   * `oracle_fixed.h` is mechanically stripped from `src/Fixed.h` (only the
//     `StdCompiler`/`StdAdaptors` includes and the serialization `CompileFunc`
//     are removed; the C4Fixed arithmetic is byte-identical to the engine).
//   * `SineTable` is the real array lifted verbatim from `src/Fixed.cpp`.
//   * `C4Random.h` is included unmodified (its only heavy include, `C4Record.h`,
//     is `#ifdef DEBUGREC` and we do not define DEBUGREC).
//   * `C4ScriptKiller.h` is the production GetKiller/SetKiller decision helper
//     called by `C4Script.cpp`; the oracle exercises that exact code.
//   * `FnEval` and DirectExec's temporary Def/LocalNamed/parent setup are
//     mechanically extracted from `C4Script.cpp` and `C4AulExec.cpp`.
//   * `C4Effect::Execute`, C4AulScriptFunc's engine-call forwarding,
//     C4AulExec's script-context setup, and `FnGetX`/`FnGetY` are
//     mechanically extracted to pin the null `this` of an
//     idCommandTarget-only object effect independently from its carrier.
//   * `C4LandscapePath.h` is the production coarse-cell traversal used by
//     `C4Landscape::_PathFree`; the oracle feeds it real PixCnt-style inputs.
//   * `C4ActionDirection.h` is the production raw-xdir direction decision used
//     by `C4Object::ExecAction` and `C4Object::SetDir`.
//   * The DFA_PUSH, DFA_PULL, and DFA_FIGHT direction blocks are mechanically
//     extracted from `C4Object::ExecAction` to pin their procedure placement.
//   * `C4ActionCallbacks.h` is the production synchronous callback sequence
//     used by `C4Object::SetAction`.
//   * `C4SolidMaskBitmap.h` is the production active-graphics bitmap selection
//     and transparency conversion used by `C4SolidMask`.
//   * `C4Object::DigOutMaterialCast` is mechanically extracted in full; a
//     minimal material/object scaffold records its spawn and RNG ledger.
//   * `C4Landscape::ClearPix`, `BlastFreePix`, and `BlastFree` are mechanically
//     extracted in full; a minimal 7x7 Surface8/material scaffold records their
//     exact scan order, material pre-counts, IFT writes, and RNG consumption.
//   * `C4Landscape::ExecuteScan` and `DoScan` are mechanically extracted in
//     full; a 6x8 Water/Ice Surface8 fixture records the exact conversion
//     cadence and ScanX cursor advancement.
//   * `C4SGame::ConvertGoals` and `C4Game::InitRules`/`InitGoals` are
//     mechanically extracted in full, together with the C4IDList operations
//     they call. The HarpoonRace fixture converts its authored RVLR rule plus
//     default StructuresNeedEnergy into the RVLR+ENRG parameter list and then
//     records the objects placed from those authoritative parameters.
//   * `C4PlayerList::GetCount` and the capacity block in `C4PlayerList::Join`
//     are mechanically extracted; a linked-player scaffold records the exact
//     admission boundary and too-many-player diagnostic argument.
//   * The complete bottom/top/side DFA_FLIGHT arms of
//     `C4Object::ContactAction`, their action helpers, and the shared
//     unresolved-flight tail are mechanically extracted; a minimal object
//     scaffold records low-speed `Disabled` contact transitions.
//   * `C4MouseControl::UpdateCursorTarget`'s OCF priority cascade is
//     mechanically extracted as a fragment; a candidate scaffold records which
//     cursor the ladder of unconditional overwrites ends on.
//   * `C4Object::AssignRemoval` is mechanically extracted in full; a
//     container/contents scaffold records its teardown order and the Status
//     re-checks between the callbacks.
//   * `C4Effect::Check` is mechanically extracted in full; a configurable
//     effect list records its negotiation order, the AnnulCalls temp bracket
//     and the Start_Deny kill.
//   * `C4Object::Enter`, `Exit` and `Collect` are mechanically extracted in
//     full; a two-object scaffold with configurable script callbacks records
//     their exact call order, rollback and post-callback Status re-checks.
//   * `C4Shape::ContactCheck` is mechanically extracted in full; a 24x16
//     material grid with configurable open borders records its per-vertex
//     contact masks, materials and counts.
//   * `C4Weather::Execute` and `C4SVal::Evaluate` are mechanically extracted in
//     full; a tick scaffold records the disaster stream and the RNG ledger per
//     tick, including the level tests that draw even at level zero.
//   * `Splash` is mechanically extracted in full; an 8x40 material grid records
//     its bubble/cast stream and the RNG ledger, including the draw-count drop
//     once its own extraction has emptied the pixel it tests.
//   * `Randomize3`/`Rnd3` are reproduced verbatim from `src/C4Random.cpp`
//     (10 trivial lines around the real `Random()`); kept in sync via the
//     provenance comment below.
//
// Regenerate the golden with `parity/oracle/gen_golden.sh`.

#include <algorithm>
#include <array>
#include <cassert>
#include <cmath>
#include <cstdint>
#include <cctype>
#include <cstdio>
#include <functional>
#include <initializer_list>
#include <optional>
#include <set>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "oracle_fixed.h" // generated from src/Fixed.h by gen_golden.sh
#include <C4Random.h>     // real engine header (no DEBUGREC)
#include <C4Constants.h>  // OCF_Alive / CNAT_Bottom
#include <C4Math.h>       // production Abs helper used by ShakeObjects
#include <C4ActionCallbacks.h> // real production SetAction callback sequence
#include <C4ActionDirection.h> // real production ExecAction/SetDir decisions
#include <C4LandscapePath.h> // real production coarse-path traversal
#include <C4ScriptKiller.h> // real production script-host helper
#include <C4SolidMaskBitmap.h> // real production active-bitmap mask sampling
#include <C4Rect.h>            // real production rect, incl. the Scaled() decl
#include <C4Components.h>     // real C4FLS_Scenario group sort order
#include <C4Strings.h>       // real declarations (and defaults) for the S* helpers

extern long SineTable[9001]; // defined by the generated sine_table.cpp

// The extracted conversion helper only formats diagnostics. Its messages are
// deliberately discarded by this bounded non-interactive fixture, so supply
// the small surface it needs without pulling the production logging stack.
namespace std
{
template <typename... Args>
string format(const char *, Args &&...)
{
    return {};
}
} // namespace std

namespace spdlog::level
{
enum level_enum
{
    warn,
};
}

template <typename... Args>
static void DebugLog(Args &&...)
{
}

// Real production C4Rect::Scaled body, lifted from src/C4Rect.cpp by
// gen_golden.sh. The truncation it performs is what maps a game-unit Picture
// rect into a scaled definition's bitmap space.
#include "rect_scaled.inc"

// --- Randomize3 / Rnd3: verbatim from src/C4Random.cpp:24-42 -----------------
static const int FRndRes = 500;
static int32_t FRndBuf3[FRndRes];
static int32_t FRndPtr3;
void Randomize3()
{
    FRndPtr3 = 0;
    for (int cnt = 0; cnt < FRndRes; cnt++) FRndBuf3[cnt] = Random(3) - 1;
}
int Rnd3()
{
    FRndPtr3++; if (FRndPtr3 == FRndRes) FRndPtr3 = 0;
    return FRndBuf3[FRndPtr3];
}

// --- Definition-commanded effect receiver + implicit GetX/GetY -------------
// Hazard's SHT1 projectile installs HitCheck on the projectile object while
// selecting the callback code by C4ID and leaving pCommandTarget null. The
// production Execute body passes that null pointer through
// C4AulScriptFunc::Exec into C4AulExec::Exec; its exact script-context setup
// leaves cthr->Obj null, so bare GetX/GetY return nil even though the carrier
// is still the callback's first argument.
namespace effect_position_oracle
{
using C4ValueInt = int32_t;
using C4ID = unsigned long;
inline constexpr int C4AUL_MAX_Par = 10;
inline constexpr int32_t C4Fx_Execute_Kill = -1;
inline constexpr int ASS_PARSED = 1;

enum class C4AulScriptStrict : uint8_t
{
    NONSTRICT = 0,
    STRICT1 = 1,
    STRICT2 = 2,
    STRICT3 = 3,
};

enum C4V_Type
{
    C4V_Any,
    C4V_Int,
    C4V_Bool,
    C4V_C4Object,
    C4V_pC4Value,
};

static const char *GetC4VName(C4V_Type type)
{
    switch (type)
    {
    case C4V_Any: return "any";
    case C4V_Int: return "int";
    case C4V_Bool: return "bool";
    case C4V_C4Object: return "object";
    case C4V_pC4Value: return "reference";
    }
    return "unknown";
}

struct C4Effect;
struct C4Def;
struct C4AulFunc;

struct C4Object
{
    C4Effect *pEffects{};
    C4Def *Def{};
    C4ID id{};
    int32_t Status{1};
    int32_t x{};
    int32_t y{};
};

struct C4AulScriptContext;
struct C4AulContext
{
    C4Object *Obj{};
    C4Def *Def{};
    C4AulScriptContext *Caller{};
};

#include "script_fn_GetX.inc"
#include "script_fn_GetY.inc"
#include "script_fn_sqrt.inc"

struct C4Value
{
    enum class Kind
    {
        Nil,
        Integer,
        Object,
    };

    Kind kind{Kind::Nil};
    int32_t integer{};
    C4Object *object{};

    int32_t getInt() const { return integer; }
    void Set(const C4Value &value) { *this = value; }
    void Set0() { *this = C4Value{}; }
    void SetInt(int32_t value) { *this = C4Value{Kind::Integer, value, nullptr}; }
    void SetBool(bool value) { SetInt(value ? 1 : 0); }
    explicit operator bool() const
    {
        return kind == Kind::Object || (kind == Kind::Integer && integer != 0);
    }
    C4V_Type GetType() const
    {
        switch (kind)
        {
        case Kind::Nil: return C4V_Any;
        case Kind::Integer: return C4V_Int;
        case Kind::Object: return C4V_C4Object;
        }
        return C4V_Any;
    }
    const char *GetTypeName() const { return GetC4VName(GetType()); }
    std::string GetDataString() const { return {}; }
    bool ConvertTo(C4V_Type type)
    {
        if (type == C4V_Any || GetType() == type)
            return true;
        return kind == Kind::Nil && type != C4V_pC4Value;
    }
};

static C4Value C4VObj(C4Object *object)
{
    return C4Value{C4Value::Kind::Object, 0, object};
}

static C4Value C4VInt(int32_t value)
{
    return C4Value{C4Value::Kind::Integer, value, nullptr};
}

static const C4Value C4VNull{};

struct PositionProbe
{
    bool callbackRan{};
    C4Object *receiver{};
    C4Object *target{};
    int32_t number{};
    int32_t time{};
    std::optional<C4ValueInt> implicitX;
    std::optional<C4ValueInt> implicitY;
    std::optional<C4ValueInt> explicitX;
    std::optional<C4ValueInt> explicitY;
};

static PositionProbe positionProbe;

struct EffectConversionProbe
{
    bool callbackRan{};
    bool receivedObjectValue{};
    bool objectIdentityMatches{};
    bool objectIdMatches{};
    bool objectEqualsCompanion{};
    bool mutateObjectOnEntry{};
};

static EffectConversionProbe effectConversionProbe;

struct C4AulParSet
{
    C4Value Par[C4AUL_MAX_Par]{};

    C4AulParSet() = default;
    C4AulParSet(
        const C4Value &par0,
        const C4Value &par1 = C4Value(),
        const C4Value &par2 = C4Value(),
        const C4Value &par3 = C4Value(),
        const C4Value &par4 = C4Value(),
        const C4Value &par5 = C4Value(),
        const C4Value &par6 = C4Value(),
        const C4Value &par7 = C4Value(),
        const C4Value &par8 = C4Value(),
        const C4Value &par9 = C4Value())
        : Par{par0, par1, par2, par3, par4, par5, par6, par7, par8, par9}
    {
    }
};

struct C4AulScript
{
    int State{ASS_PARSED};
    C4Def *Def{};
    C4AulScriptStrict Strict{C4AulScriptStrict::STRICT2};
    C4AulFunc *callback{};

    C4AulFunc *GetFuncRecursive(const char *)
    {
        return callback;
    }
};

enum class CallbackProbeKind
{
    Position,
    EffectConversion,
};

struct C4AulBCC
{
    CallbackProbeKind probeKind{CallbackProbeKind::Position};
    int probeParameter{};
    int comparisonParameter{-1};
    C4Object *expectedObject{};
};

struct C4AulScriptFunc;

struct C4AulScriptContext : C4AulContext
{
    C4Value *Return{};
    C4Value *Pars{};
    C4Value *Vars{};
    C4AulScriptFunc *Func{};
    bool TemporaryScript{};
    C4AulBCC *CPos{};
};

struct C4AulFunc
{
    struct NameValue
    {
        const char *value{};
    };

    NameValue Name{};
    int parCount{};
    std::array<C4V_Type, C4AUL_MAX_Par> parTypes{};

    virtual ~C4AulFunc() = default;
    virtual int GetParCount() { return parCount; }
    virtual const C4V_Type *GetParType() { return parTypes.data(); }
    virtual C4Value Exec(
        C4Object *pObj = nullptr,
        const C4AulParSet &parameters = C4AulParSet{},
        bool fPassErrors = false,
        bool nonStrict3WarnConversionOnly = false,
        bool convertNilToIntBool = true) = 0;
};

static const char *operator+(const C4AulFunc::NameValue &name)
{
    return name.value;
}

struct C4Def
{
    C4AulScript Script{};
};

struct C4ValueMapNames
{
    int iSize{};
};

struct C4AulScriptFunc : C4AulFunc
{
    C4AulScript *Owner{};
    C4AulScript *pOrgScript{};
    C4AulBCC *Code{};
    C4ValueMapNames VarNamed{};

    bool HasStrictNil() const noexcept;
    C4Value Exec(
        C4Object *pObj = nullptr,
        const C4AulParSet &pPars = C4AulParSet{},
        bool fPassErrors = false,
        bool nonStrict3WarnConversionOnly = false,
        bool convertNilToIntBool = true) override;
};

class C4AulError
{
public:
    virtual ~C4AulError() = default;
    virtual void show() const {}
};

class C4AulExecError : public C4AulError
{
public:
    C4AulExecError(C4Object *, std::string_view) {}
};

#include "aul_script_func_has_strict_nil.inc"
#include "aul_parameter_conversion.inc"

struct C4AulExec
{
    std::array<C4Value, 128> valueStack{};
    C4Value *pCurVal{valueStack.data()};
    C4AulScriptContext currentContext{};

    void Reset()
    {
        pCurVal = valueStack.data();
        currentContext = C4AulScriptContext{};
    }

    void PushValue(const C4Value &value)
    {
        ++pCurVal;
        pCurVal->Set(value);
    }

    void PushNullVals(int count)
    {
        while (count-- > 0)
            PushValue(C4VNull);
    }

    void PushContext(const C4AulScriptContext &context)
    {
        currentContext = context;
    }

    C4Value Exec(
        C4AulScriptFunc *pSFunc,
        C4Object *pObj,
        const C4Value *pnPars,
        bool fPassErrors,
        bool fTemporaryScript = false);

    C4Value Exec(C4AulBCC *code, bool)
    {
        if (code->probeKind == CallbackProbeKind::EffectConversion)
        {
            const C4Value &value = currentContext.Pars[code->probeParameter];
            C4Object *target = value.object;
            effectConversionProbe.callbackRan = true;
            effectConversionProbe.receivedObjectValue =
                value.GetType() == C4V_C4Object;
            effectConversionProbe.objectIdentityMatches =
                target == code->expectedObject;
            effectConversionProbe.objectIdMatches =
                target && code->expectedObject &&
                target->id == code->expectedObject->id;
            effectConversionProbe.objectEqualsCompanion =
                code->comparisonParameter >= 0 &&
                value.GetType() == C4V_C4Object &&
                currentContext.Pars[code->comparisonParameter].GetType() == C4V_C4Object &&
                target == currentContext.Pars[code->comparisonParameter].object;
            if (effectConversionProbe.mutateObjectOnEntry && target)
                target->x = 999;
            return C4VInt(0);
        }

        C4Object *target = currentContext.Pars[0].object;

        positionProbe.callbackRan = true;
        positionProbe.receiver = currentContext.Obj;
        positionProbe.target = target;
        positionProbe.number = currentContext.Pars[1].integer;
        positionProbe.time = currentContext.Pars[2].integer;
        positionProbe.implicitX = FnGetX(&currentContext, nullptr);
        positionProbe.implicitY = FnGetY(&currentContext, nullptr);
        positionProbe.explicitX = FnGetX(&currentContext, target);
        positionProbe.explicitY = FnGetY(&currentContext, target);
        return C4VInt(0);
    }
};

static C4AulExec AulExec;

#include "aul_script_func_exec.inc"
#include "aul_exec_script_context.inc"

struct C4Effect
{
    C4AulFunc::NameValue Name{};
    C4Object *pCommandTarget{};
    C4ID idCommandTarget{};
    int32_t iPriority{100};
    int32_t iTime{};
    int32_t iIntervall{1};
    int32_t iNumber{1};
    C4Effect *pNext{};
    C4AulFunc *pFnTimer{};

    bool IsDead() { return !iPriority; }
    void Kill(C4Object *) { iPriority = 0; }
    void Execute(C4Object *pObj);
    C4Value DoCall(
        C4Object *pObj,
        const char *szFn,
        const C4Value &rVal1,
        const C4Value &rVal2,
        const C4Value &rVal3,
        const C4Value &rVal4,
        const C4Value &rVal5,
        const C4Value &rVal6,
        const C4Value &rVal7,
        bool passErrors,
        bool convertNilToIntBool);
};

struct DefinitionRegistry
{
    C4Def *definition{};

    C4Def *ID2Def(C4ID)
    {
        return definition;
    }
};

struct GameState
{
    C4Effect *pGlobalEffects{};
    DefinitionRegistry Defs{};
    C4AulScript ScriptEngine{};
};

static GameState Game;

inline constexpr char PSF_FxCustom[] = "";

#include "effect_execute.inc"
#include "effect_do_call.inc"

static void printOptional(std::optional<C4ValueInt> value)
{
    if (value)
        printf("%d", *value);
    else
        printf("null");
}

static void printDefinitionCommandedEffectPositionCase()
{
    positionProbe = PositionProbe{};
    Game = GameState{};

    C4Object carrier;
    carrier.x = 320;
    carrier.y = -50;

    C4AulScript callbackOwner;
    C4AulBCC callbackCode;
    C4AulScriptFunc timer;
    timer.Owner = &callbackOwner;
    timer.pOrgScript = &callbackOwner;
    timer.Code = &callbackCode;
    C4Effect effect;
    effect.pCommandTarget = nullptr;
    effect.idCommandTarget = 0x424f5250UL; // little-endian "PROB"
    effect.pFnTimer = &timer;
    carrier.pEffects = &effect;

    effect.Execute(&carrier);

    printf("\"definition_commanded_effect_position\":{"
           "\"carrier_x\":%d,\"carrier_y\":%d,"
           "\"has_id_command_target\":%d,\"command_target_is_null\":%d,"
           "\"callback_ran\":%d,\"callback_receiver_is_null\":%d,"
           "\"callback_target_is_carrier\":%d,\"number\":%d,\"time\":%d,"
           "\"implicit_x\":",
           carrier.x, carrier.y, effect.idCommandTarget != 0,
           effect.pCommandTarget == nullptr, positionProbe.callbackRan,
           positionProbe.receiver == nullptr,
           positionProbe.target == &carrier, positionProbe.number,
           positionProbe.time);
    printOptional(positionProbe.implicitX);
    printf(",\"implicit_y\":");
    printOptional(positionProbe.implicitY);
    printf(",\"explicit_x\":");
    printOptional(positionProbe.explicitX);
    printf(",\"explicit_y\":");
    printOptional(positionProbe.explicitY);
    printf("}");
}

struct EffectConversionResult
{
    bool callbackRan{};
    bool receivedObjectValue{};
    bool objectIdentityMatches{};
    bool objectIdMatches{};
    bool objectEqualsCompanion{};
    bool carrierMutated{};
};

static EffectConversionResult runEffectCallbackConversionCase(
    C4AulScriptStrict strict,
    C4V_Type declaredType,
    bool callbackWouldMutateCarrier)
{
    effectConversionProbe = EffectConversionProbe{};
    effectConversionProbe.mutateObjectOnEntry = callbackWouldMutateCarrier;
    Game = GameState{};
    AulExec.Reset();

    C4Object carrier;
    carrier.id = 0x544d4850UL; // TMHP fixture definition ID
    carrier.x = 37;
    C4AulScript callbackOwner;
    callbackOwner.Strict = strict;
    C4AulBCC callbackCode;
    callbackCode.probeKind = CallbackProbeKind::EffectConversion;
    callbackCode.expectedObject = &carrier;
    C4AulScriptFunc timer;
    timer.Name.value = "FxOracleTimer";
    timer.Owner = &callbackOwner;
    timer.pOrgScript = &callbackOwner;
    timer.Code = &callbackCode;
    timer.parCount = 1;
    timer.parTypes[0] = declaredType;
    C4Effect effect;
    effect.pFnTimer = &timer;
    carrier.pEffects = &effect;

    // The production Execute body supplies C4VObj(carrier) to the callback
    // and its engine-call entry passes true for warning-only conversion.
    effect.Execute(&carrier);
    return {
        effectConversionProbe.callbackRan,
        effectConversionProbe.receivedObjectValue,
        effectConversionProbe.objectIdentityMatches,
        effectConversionProbe.objectIdMatches,
        effectConversionProbe.objectEqualsCompanion,
        carrier.x != 37,
    };
}

static EffectConversionResult runEffectCallConversionCase(
    C4AulScriptStrict strict,
    C4V_Type declaredExtraType,
    bool callbackWouldMutateExtra)
{
    effectConversionProbe = EffectConversionProbe{};
    effectConversionProbe.mutateObjectOnEntry = callbackWouldMutateExtra;
    Game = GameState{};
    AulExec.Reset();

    C4Object carrier;
    carrier.id = 0x45434850UL; // ECHP fixture definition ID
    carrier.x = 37;
    C4Def callbackDefinition;
    callbackDefinition.Script.Def = &callbackDefinition;
    callbackDefinition.Script.Strict = strict;
    C4AulBCC callbackCode;
    callbackCode.probeKind = CallbackProbeKind::EffectConversion;
    callbackCode.probeParameter = 2;
    callbackCode.comparisonParameter = 0;
    callbackCode.expectedObject = &carrier;
    C4AulScriptFunc callback;
    callback.Name.value = "FxOracleProbe";
    callback.Owner = &callbackDefinition.Script;
    callback.pOrgScript = &callbackDefinition.Script;
    callback.Code = &callbackCode;
    callback.parCount = 3;
    callback.parTypes[0] = C4V_C4Object;
    callback.parTypes[1] = C4V_Int;
    callback.parTypes[2] = declaredExtraType;
    callbackDefinition.Script.callback = &callback;
    Game.Defs.definition = &callbackDefinition;
    C4Effect effect;
    effect.Name.value = "Oracle";
    effect.idCommandTarget = 1;

    // This is the exact EffectCall route: extracted DoCall resolves the
    // command-id script, copies the extra C4Value into C4AulParSet, and calls
    // the callback with nonStrict3WarnConversionOnly=true.
    try
    {
        effect.DoCall(
            &carrier,
            "Probe",
            C4VObj(&carrier),
            C4VNull,
            C4VNull,
            C4VNull,
            C4VNull,
            C4VNull,
            C4VNull,
            true,
            true);
    }
    catch (const C4AulError &)
    {
        // FnEffectCall asks DoCall to pass errors. STRICT3 must therefore
        // reject before the probe body receives or mutates the extra object.
    }
    return {
        effectConversionProbe.callbackRan,
        effectConversionProbe.receivedObjectValue,
        effectConversionProbe.objectIdentityMatches,
        effectConversionProbe.objectIdMatches,
        effectConversionProbe.objectEqualsCompanion,
        carrier.x != 37,
    };
}

static void printEffectCallbackConversionCase()
{
    const auto preStrict3 = runEffectCallbackConversionCase(
        C4AulScriptStrict::STRICT2, C4V_Int, true);
    const auto strict3 = runEffectCallbackConversionCase(
        C4AulScriptStrict::STRICT3, C4V_Int, false);
    const auto strict3Reference = runEffectCallbackConversionCase(
        C4AulScriptStrict::STRICT3, C4V_pC4Value, true);
    const auto effectCallPreStrict3 = runEffectCallConversionCase(
        C4AulScriptStrict::STRICT2, C4V_Int, true);
    const auto effectCallStrict3 = runEffectCallConversionCase(
        C4AulScriptStrict::STRICT3, C4V_Int, false);
    const auto effectCallStrict3Reference = runEffectCallConversionCase(
        C4AulScriptStrict::STRICT3, C4V_pC4Value, true);

    printf("\"effect_callback_conversion\":{"
           "\"pre_strict3_callback_ran\":%d,"
           "\"pre_strict3_original_object\":%d,"
           "\"strict3_rejected\":%d,"
           "\"strict3_callback_ran\":%d,"
           "\"strict3_reference_rejected\":%d,"
           "\"strict3_reference_callback_ran\":%d,"
           "\"strict3_reference_object_mutated\":%d,"
           "\"effect_call_pre_strict3_callback_ran\":%d,"
           "\"effect_call_pre_strict3_type_is_object\":%d,"
           "\"effect_call_pre_strict3_identity_matches\":%d,"
           "\"effect_call_pre_strict3_id_matches\":%d,"
           "\"effect_call_pre_strict3_target_equals_extra\":%d,"
           "\"effect_call_pre_strict3_object_mutated\":%d,"
           "\"effect_call_strict3_rejected\":%d,"
           "\"effect_call_strict3_callback_ran\":%d,"
           "\"effect_call_strict3_reference_rejected\":%d,"
           "\"effect_call_strict3_reference_callback_ran\":%d,"
           "\"effect_call_strict3_reference_object_mutated\":%d}",
           preStrict3.callbackRan,
           preStrict3.receivedObjectValue,
           !strict3.callbackRan,
           strict3.callbackRan,
           !strict3Reference.callbackRan,
           strict3Reference.callbackRan,
           strict3Reference.carrierMutated,
           effectCallPreStrict3.callbackRan,
           effectCallPreStrict3.receivedObjectValue,
           effectCallPreStrict3.objectIdentityMatches,
           effectCallPreStrict3.objectIdMatches,
           effectCallPreStrict3.objectEqualsCompanion,
           effectCallPreStrict3.carrierMutated,
           !effectCallStrict3.callbackRan,
           effectCallStrict3.callbackRan,
           !effectCallStrict3Reference.callbackRan,
           effectCallStrict3Reference.callbackRan,
           effectCallStrict3Reference.carrierMutated);
}
} // namespace effect_position_oracle

// --- BlastFree: exact production bodies ------------------------------------
// gen_golden.sh extracts ClearPix, BlastFreePix, and BlastFree unchanged from
// src/C4Landscape.cpp. This scaffold models only the fields/callees those
// bodies touch. Material ids are Earth=0, Granite=1, Rock=2, Tunnel=3. The
// texmap deliberately gives Rock and Tunnel duplicate slots: BlastShiftTo
// names the second Rock texture, while ClearPix must use Tunnel's second,
// DefaultMatTex slot rather than the first slot carrying that material.
inline constexpr uint8_t IFT_Oracle = 0x80;
inline constexpr int32_t C4MaxMaterial_Oracle = 125;
inline constexpr int32_t MNone_OracleLandscape = -1;
inline constexpr int32_t C4ID_None_Oracle = 0;
inline constexpr int32_t Earth_Oracle = 0;
inline constexpr int32_t Granite_Oracle = 1;
inline constexpr int32_t Rock_Oracle = 2;
inline constexpr int32_t Tunnel_Oracle = 3;
inline constexpr int32_t Water_Oracle = 4;
inline constexpr int32_t Ice_Oracle = 5;
inline constexpr uint8_t EarthPix_Oracle = 1;
inline constexpr uint8_t GranitePix_Oracle = 2;
inline constexpr uint8_t RockDefaultPix_Oracle = 3;
inline constexpr uint8_t TunnelFirstPix_Oracle = 4;
inline constexpr uint8_t RockShiftPix_Oracle = 5;
inline constexpr uint8_t TunnelDefaultPix_Oracle = 6;
inline constexpr uint8_t WaterPix_Oracle = 7;
inline constexpr uint8_t IcePix_Oracle = 8;

struct C4MaterialOracle
{
    int32_t Dig2Object{C4ID_None_Oracle};
    int32_t Dig2ObjectRatio{};
    int32_t Dig2ObjectOnRequestOnly{};
    int32_t BlastFree{};
    int32_t BlastShiftTo{};
    int32_t Blast2Object{C4ID_None_Oracle};
    int32_t Blast2ObjectRatio{};
    int32_t Blast2PXSRatio{};
    int32_t DefaultMatTex{};
    int32_t BelowTempConvert{};
    int32_t BelowTempConvertDir{};
    int32_t BelowTempConvertTo{};
    int32_t AboveTempConvert{};
    int32_t AboveTempConvertDir{};
    int32_t AboveTempConvertTo{};
    int32_t TempConvStrength{};
};

struct C4MaterialMapOracle
{
    int32_t Num{};
    C4MaterialOracle Map[C4MaxMaterial_Oracle]{};
};

struct C4PXSOracle
{
    void Cast(int32_t, int32_t, int32_t, int32_t, int32_t) {}
};

struct C4WeatherOracle
{
    int32_t Temperature{};
    int32_t GetTemperature() const { return Temperature; }
};

struct C4TexMapEntryOracle
{
    int32_t MaterialIndex{MNone_OracleLandscape};
    int32_t GetMaterialIndex() const { return MaterialIndex; }
};

struct C4TextureMapOracle
{
    C4TexMapEntryOracle Entries[256]{};

    C4TexMapEntryOracle *GetEntry(int32_t texture)
    {
        if (texture < 0 || texture >= 256) return nullptr;
        return &Entries[texture];
    }
};

struct C4GameLandscapeOracle
{
    C4MaterialMapOracle Material;
    C4PXSOracle PXS;
    C4WeatherOracle Weather;
    C4TextureMapOracle TextureMap;
    void BlastCastObjects(int32_t, void *, int32_t, int32_t, int32_t, int32_t) {}
};

static C4GameLandscapeOracle GameLandscapeOracle;

class C4Landscape
{
public:
    int32_t Width{7};
    int32_t Height{7};
    int32_t ScanX{};
    int32_t ScanSpeed{2};
    uint32_t MatCount[C4MaxMaterial_Oracle]{};
    int32_t BlastMatCount[C4MaxMaterial_Oracle]{};
    uint8_t Pixels[8 * 8]{};
    int32_t Pix2Mat[256]{};

    C4Landscape()
    {
        for (int32_t &material : Pix2Mat) material = MNone_OracleLandscape;
        Pix2Mat[EarthPix_Oracle] = Pix2Mat[EarthPix_Oracle | IFT_Oracle] = Earth_Oracle;
        Pix2Mat[GranitePix_Oracle] = Pix2Mat[GranitePix_Oracle | IFT_Oracle] = Granite_Oracle;
        Pix2Mat[RockDefaultPix_Oracle] =
            Pix2Mat[RockDefaultPix_Oracle | IFT_Oracle] = Rock_Oracle;
        Pix2Mat[TunnelFirstPix_Oracle] =
            Pix2Mat[TunnelFirstPix_Oracle | IFT_Oracle] = Tunnel_Oracle;
        Pix2Mat[RockShiftPix_Oracle] =
            Pix2Mat[RockShiftPix_Oracle | IFT_Oracle] = Rock_Oracle;
        Pix2Mat[TunnelDefaultPix_Oracle] =
            Pix2Mat[TunnelDefaultPix_Oracle | IFT_Oracle] = Tunnel_Oracle;
        Pix2Mat[WaterPix_Oracle] = Pix2Mat[WaterPix_Oracle | IFT_Oracle] = Water_Oracle;
        Pix2Mat[IcePix_Oracle] = Pix2Mat[IcePix_Oracle | IFT_Oracle] = Ice_Oracle;
    }

    uint8_t GetPix(int32_t x, int32_t y) const
    {
        if (x < 0 || y < 0 || x >= Width || y >= Height) return 0;
        return Pixels[y * Width + x];
    }

    int32_t GetMat(int32_t x, int32_t y) const
    {
        return Pix2Mat[GetPix(x, y)];
    }

    uint8_t _GetPix(int32_t x, int32_t y) const { return Pixels[y * Width + x]; }

    int32_t _GetMat(int32_t x, int32_t y) const { return Pix2Mat[_GetPix(x, y)]; }

    bool SetPix(int32_t x, int32_t y, uint8_t pixel)
    {
        if (x < 0 || y < 0 || x >= Width || y >= Height) return false;
        const uint8_t oldPixel = Pixels[y * Width + x];
        const int32_t oldMaterial = Pix2Mat[oldPixel];
        const int32_t newMaterial = Pix2Mat[pixel];
        if (oldMaterial != newMaterial)
        {
            if (oldPixel && oldMaterial >= 0) --MatCount[oldMaterial];
            if (pixel && newMaterial >= 0) ++MatCount[newMaterial];
        }
        Pixels[y * Width + x] = pixel;
        return true;
    }

    void ClearBlastMatCount()
    {
        for (int32_t &count : BlastMatCount) count = 0;
    }

    void CheckInstabilityRange(int32_t, int32_t) {}

    bool ClearPix(int32_t tx, int32_t ty);
    int32_t BlastFreePix(int32_t tx, int32_t ty, int32_t grade, int32_t iBlastSize);
    void BlastFree(int32_t tx, int32_t ty, int32_t rad, int32_t grade, int32_t iByPlayer);
    void ExecuteScan();
    int32_t DoScan(int32_t cx, int32_t cy, int32_t mat, int32_t dir);
};

#define IFT IFT_Oracle
#define C4MaxMaterial C4MaxMaterial_Oracle
#define C4ID_None C4ID_None_Oracle
#define Game GameLandscapeOracle
#define GBackIFT(x, y) (GetPix((x), (y)) & IFT_Oracle)
inline bool MatValid(int32_t material)
{
    return material >= 0 && material < GameLandscapeOracle.Material.Num;
}
inline uint8_t MatTex2PixCol(int32_t texture) { return static_cast<uint8_t>(texture); }
inline uint8_t Mat2PixColDefault(int32_t material)
{
    return static_cast<uint8_t>(GameLandscapeOracle.Material.Map[material].DefaultMatTex);
}
static int32_t MTunnel = Tunnel_Oracle;
#include "landscape_clear_pix.inc"
#include "landscape_blast_free_pix.inc"
#include "landscape_blast_free.inc"
#define GBackWdt Width
#define GBackHgt Height
#define SBackPix(x, y, pixel) SetPix((x), (y), (pixel))
#define PixCol2Mat(pixel) (Pix2Mat[static_cast<uint8_t>(pixel)])
#define PixColIFT(pixel) (static_cast<uint8_t>(pixel) & IFT_Oracle)
#define PRETTY_TEMP_CONV
#include "landscape_execute_scan.inc"
#include "landscape_do_scan.inc"
#undef PRETTY_TEMP_CONV
#undef PixColIFT
#undef PixCol2Mat
#undef SBackPix
#undef GBackHgt
#undef GBackWdt
#undef GBackIFT
#undef Game
#undef C4ID_None
#undef C4MaxMaterial
#undef IFT

static void printBlastFreeCase()
{
    C4GameLandscapeOracle &game = GameLandscapeOracle;
    game = C4GameLandscapeOracle{};
    game.Material.Num = 4;
    game.Material.Map[Earth_Oracle].BlastFree = 1;
    game.Material.Map[Earth_Oracle].DefaultMatTex = EarthPix_Oracle;
    game.Material.Map[Granite_Oracle].BlastShiftTo = RockShiftPix_Oracle;
    game.Material.Map[Granite_Oracle].DefaultMatTex = GranitePix_Oracle;
    game.Material.Map[Rock_Oracle].BlastFree = 1;
    game.Material.Map[Rock_Oracle].DefaultMatTex = RockDefaultPix_Oracle;
    game.Material.Map[Tunnel_Oracle].DefaultMatTex = TunnelDefaultPix_Oracle;

    C4Landscape landscape;
    for (int32_t y = 0; y < landscape.Height; ++y)
        for (int32_t x = 0; x < landscape.Width; ++x)
        {
            uint8_t pixel = ((x + y) % 2 == 0) ? EarthPix_Oracle : GranitePix_Oracle;
            if ((x + 2 * y) % 3 != 0) pixel |= IFT_Oracle;
            landscape.Pixels[y * landscape.Width + x] = pixel;
        }

    uint8_t initial[7 * 7];
    for (int32_t index = 0; index < 7 * 7; ++index) initial[index] = landscape.Pixels[index];

    FixedRandom(2);
    Randomize3();
    const int32_t countBefore = RandomCount;
    const uint32_t holdBefore = RandomHold;
    const int32_t rnd3Before = FRndPtr3;
    landscape.BlastFree(3, 3, 3, 1, 7);

    printf("\"blast_free\":{\"seed\":2,\"width\":7,\"height\":7,"
           "\"x\":3,\"y\":3,\"radius\":3,\"grade\":1,\"controller\":7,"
           "\"material_bytes\":{\"earth\":%u,\"granite\":%u,"
           "\"rock_default\":%u,\"tunnel_first\":%u,"
           "\"rock_shift\":%u,\"tunnel_default\":%u},"
           "\"initial_bytes\":[",
           static_cast<unsigned int>(EarthPix_Oracle),
           static_cast<unsigned int>(GranitePix_Oracle),
           static_cast<unsigned int>(RockDefaultPix_Oracle),
           static_cast<unsigned int>(TunnelFirstPix_Oracle),
           static_cast<unsigned int>(RockShiftPix_Oracle),
           static_cast<unsigned int>(TunnelDefaultPix_Oracle));
    for (int32_t index = 0; index < 7 * 7; ++index)
    {
        if (index) printf(",");
        printf("%u", static_cast<unsigned int>(initial[index]));
    }
    printf("],\"pre_counts\":{\"earth\":%d,\"granite\":%d,\"rock\":%d,\"tunnel\":%d},"
           "\"rng_before\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},"
           "\"rng_after\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},"
           "\"final_bytes\":[",
           landscape.BlastMatCount[Earth_Oracle], landscape.BlastMatCount[Granite_Oracle],
           landscape.BlastMatCount[Rock_Oracle], landscape.BlastMatCount[Tunnel_Oracle],
           countBefore, holdBefore, rnd3Before, RandomCount, RandomHold, FRndPtr3);
    for (int32_t index = 0; index < 7 * 7; ++index)
    {
        if (index) printf(",");
        printf("%u", static_cast<unsigned int>(landscape.Pixels[index]));
    }

    C4Landscape zeroRadiusLandscape;
    zeroRadiusLandscape.Pixels[3 * zeroRadiusLandscape.Width + 3] =
        EarthPix_Oracle | IFT_Oracle;
    FixedRandom(17);
    Randomize3();
    const int32_t zeroCountBefore = RandomCount;
    const uint32_t zeroHoldBefore = RandomHold;
    zeroRadiusLandscape.BlastFree(3, 3, 0, 1, 7);
    printf("],\"zero_radius\":{\"seed\":17,\"x\":3,\"y\":3,"
           "\"initial_byte\":%u,\"pre_count\":%d,\"final_byte\":%u,"
           "\"rng_before\":{\"count\":%d,\"hold\":%u},"
           "\"rng_after\":{\"count\":%d,\"hold\":%u}}}",
           static_cast<unsigned int>(EarthPix_Oracle | IFT_Oracle),
           zeroRadiusLandscape.BlastMatCount[Earth_Oracle],
           static_cast<unsigned int>(
               zeroRadiusLandscape.Pixels[3 * zeroRadiusLandscape.Width + 3]),
           zeroCountBefore, zeroHoldBefore, RandomCount, RandomHold);
}

static void printLandscapeScanCase()
{
    C4GameLandscapeOracle &game = GameLandscapeOracle;
    game = C4GameLandscapeOracle{};
    game.Material.Num = 6;
    game.Material.Map[Water_Oracle].BelowTempConvert = -10;
    game.Material.Map[Water_Oracle].BelowTempConvertDir = 0;
    game.Material.Map[Water_Oracle].BelowTempConvertTo = IcePix_Oracle;
    game.Material.Map[Water_Oracle].TempConvStrength = 3;
    game.Material.Map[Water_Oracle].DefaultMatTex = WaterPix_Oracle;
    game.Material.Map[Ice_Oracle].DefaultMatTex = IcePix_Oracle;
    game.TextureMap.Entries[WaterPix_Oracle].MaterialIndex = Water_Oracle;
    game.TextureMap.Entries[IcePix_Oracle].MaterialIndex = Ice_Oracle;
    game.Weather.Temperature = -20;

    C4Landscape landscape;
    landscape.Width = 6;
    landscape.Height = 8;
    landscape.ScanX = 0;
    landscape.ScanSpeed = 2;
    for (int32_t y = 0; y < 6; ++y)
        for (int32_t x = 0; x < landscape.Width; ++x)
            landscape.Pixels[y * landscape.Width + x] = WaterPix_Oracle;
    landscape.MatCount[Water_Oracle] = 36;

    printf("\"landscape_scan\":{\"width\":6,\"height\":8,\"water_depth\":6,"
           "\"temperature\":-20,"
           "\"below_temperature\":-10,\"direction\":0,\"strength\":3,"
           "\"scan_speed\":2,\"water_byte\":%u,\"ice_byte\":%u,\"states\":[",
           static_cast<unsigned int>(WaterPix_Oracle),
           static_cast<unsigned int>(IcePix_Oracle));
    for (int32_t frame = 0; frame <= 6; ++frame)
    {
        if (frame) printf(",");
        int32_t water = 0, ice = 0;
        for (int32_t index = 0; index < landscape.Width * landscape.Height; ++index)
        {
            water += landscape.Pixels[index] == WaterPix_Oracle;
            ice += landscape.Pixels[index] == IcePix_Oracle;
        }
        printf("{\"frame\":%d,\"scan_x\":%d,\"water\":%d,\"ice\":%d}",
               frame, landscape.ScanX, water, ice);
        if (frame < 6) landscape.ExecuteScan();
    }
    printf("]}");
}

// --- ShakeObjects + raw Fling: exact production bodies ----------------------
// gen_golden.sh extracts the two method bodies unchanged from src/. Only the
// minimal fields those methods touch are modeled here. Tumble/Jump return false
// so both C++ and Rust exercise C4Object::Fling's raw-velocity fallback.
inline constexpr int32_t C4D_Vehicle_Oracle = 1 << 2; // C4Def.h:46
inline constexpr int32_t C4D_Living_Oracle = 1 << 3;
inline constexpr int32_t C4D_Object_Oracle = 1 << 4;
inline constexpr int32_t DFA_FLOAT_Oracle = 13;             // C4Def.h:443
inline constexpr int32_t C4FxCall_DmgBlast_Oracle = 1;      // C4Effects.h:54
inline constexpr int32_t C4FxCall_EngBlast_Oracle = 33;     // C4Effects.h:60
inline constexpr int32_t MNone_Oracle = -1;
inline constexpr int32_t DIR_Left = 0;
inline constexpr int32_t DIR_Right = 1;
inline constexpr int32_t ContactActionFlight_Oracle = 0;
inline constexpr int32_t ContactActionFlatUp_Oracle = 1;
inline constexpr int32_t ContactActionKneelDown_Oracle = 2;
inline constexpr int32_t ContactActionWalk_Oracle = 3;
inline constexpr int32_t ContactActionTumble_Oracle = 4;
inline constexpr int32_t ContactActionScale_Oracle = 5;
inline constexpr int32_t ContactActionHangle_Oracle = 6;
int32_t MVehic = 1;

inline bool MatVehicle(int32_t material) { return material == MVehic; }

struct C4Object;

struct C4ObjectLink
{
    C4Object *Obj{};
    C4ObjectLink *Next{};
};

struct C4ObjectListOracle
{
    C4ObjectLink *First{};
};

struct C4PhysicalInfo
{
    int32_t CanScale{};
    int32_t CanHangle{};
};

struct C4Object
{
    struct ActionState
    {
        uint8_t t_attach{};
        int32_t Dir{};
        // C4Object.h:440 reads ActMap through Action.Act; ActIdle is -1.
        int32_t Act{-1};
    } Action;
    struct ShapeState
    {
        int32_t AttachMat{MNone_Oracle};
        int32_t x{};
        int32_t y{};
        int32_t Wdt{};
        int32_t Hgt{};
    } Shape;

    // Only the C4Def fields BlastObjects and Blast read.
    struct DefOracle
    {
        int32_t NoHorizontalMove{};
        int32_t Grab{};
        int32_t BlastIncinerate{};
        struct ActMapEntry
        {
            int32_t Procedure{-1};
        } ActMap[4];
    } *Def{};

    int32_t Mass{};
    C4Object *pLayer{};
    int32_t Alive{};
    int32_t Damage{};

    // Blast's callees are recorded rather than run: their real bodies reach
    // rules, effects and script, none of which this section scaffolds. What is
    // compared is which object received which call, with which argument, in
    // what order.
    int32_t DamageCalls{};
    int32_t DamageSum{};
    int32_t EnergyCalls{};
    int32_t EnergySum{};
    int32_t IncinerateCalls{};

    void DoDamage(int32_t change, int32_t, int32_t)
    {
        ++DamageCalls;
        DamageSum += change;
        Damage += change;
    }

    void DoEnergy(int32_t change, bool, int32_t, int32_t)
    {
        ++EnergyCalls;
        EnergySum += change;
    }

    void Incinerate(int32_t, bool) { ++IncinerateCalls; }

    void Blast(int32_t iLevel, int32_t iCausedBy);

    int32_t MaterialContents[C4MaxMaterial_Oracle]{};
    int32_t Status{};
    C4Object *Contained{};
    int32_t Category{};
    int32_t x{};
    int32_t y{};
    uint32_t OCF{};
    int32_t Controller{-1};
    int32_t LastEnergyLossCausePlayer{-1};
    C4Fixed xdir{Fix0};
    C4Fixed ydir{Fix0};
    int32_t Mobile{};
    int32_t ContactAction{ContactActionFlight_Oracle};
    int32_t ContactActionXdirBeforeFlightStuck{};
    int32_t ContactActionYdirBeforeFlightStuck{};

    void UpdatLastEnergyLossCause(int32_t cause)
    {
        if (cause != Controller || LastEnergyLossCausePlayer < 0)
            LastEnergyLossCausePlayer = cause;
    }

    void Fling(C4Fixed txdir, C4Fixed tydir, bool addSpeed, int32_t causedBy);
    void DigOutMaterialCast(bool request);

    bool SetActionByName(const char *name)
    {
        const std::string_view action{name};
        if (action == "FlatUp") ContactAction = ContactActionFlatUp_Oracle;
        else if (action == "KneelDown") ContactAction = ContactActionKneelDown_Oracle;
        else if (action == "Walk") ContactAction = ContactActionWalk_Oracle;
        else if (action == "Tumble") ContactAction = ContactActionTumble_Oracle;
        else if (action == "Scale") ContactAction = ContactActionScale_Oracle;
        else if (action == "Hangle") ContactAction = ContactActionHangle_Oracle;
        else return false;
        return true;
    }

    void SetDir(int32_t dir) { Action.Dir = dir; }
    void ForcePosition(int32_t new_x, int32_t new_y) { x = new_x; y = new_y; }
    void ContactActionBottomFlight(int32_t fDisabled);
    void ContactActionTopFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical);
    void ContactActionLeftFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical);
    void ContactActionRightFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical);
};

// Exact source text generated from C4ObjectCom.cpp. Keeping the helpers exact
// makes a successful gate observable as the real FlatUp action/velocity result.
#include "object_action_walk.inc"
#include "object_action_kneel.inc"
#include "object_action_flat.inc"
#define ObjectActionTumble ObjectActionTumbleContactOracle
#include "object_action_tumble.inc"
#undef ObjectActionTumble
#include "object_action_scale.inc"
#include "object_action_hangle.inc"

// Exact first DFA_FLIGHT arm from C4Object::ContactAction. The scaffold fixes
// iProcedure to flight and supplies only the locals surrounding the extracted
// switch arm; the production OR condition itself is compiled verbatim.
#define DFA_FLIGHT 1
#define ObjectActionTumble ObjectActionTumbleContactOracle
void C4Object::ContactActionBottomFlight(int32_t fDisabled)
{
    C4Fixed last_xdir;
    int32_t iProcedure = DFA_FLIGHT;
    switch (iProcedure)
    {
#include "contact_action_bottom_flight.inc"
    default: return;
    }
}

void C4Object::ContactActionTopFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical)
{
    int32_t iProcedure = DFA_FLIGHT;
    uint32_t t_contact = CNAT_Top;
    switch (iProcedure)
    {
#include "contact_action_top_flight.inc"
    default: return;
    }
    ContactActionXdirBeforeFlightStuck = xdir.val;
    ContactActionYdirBeforeFlightStuck = ydir.val;
#include "contact_action_flight_stuck.inc"
}

void C4Object::ContactActionLeftFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical)
{
    int32_t iProcedure = DFA_FLIGHT;
    uint32_t t_contact = CNAT_Left;
    switch (iProcedure)
    {
#include "contact_action_left_flight.inc"
    default: return;
    }
    ContactActionXdirBeforeFlightStuck = xdir.val;
    ContactActionYdirBeforeFlightStuck = ydir.val;
#include "contact_action_flight_stuck.inc"
}

void C4Object::ContactActionRightFlight(int32_t fDisabled, C4PhysicalInfo *pPhysical)
{
    int32_t iProcedure = DFA_FLIGHT;
    uint32_t t_contact = CNAT_Right;
    switch (iProcedure)
    {
#include "contact_action_right_flight.inc"
    default: return;
    }
    ContactActionXdirBeforeFlightStuck = xdir.val;
    ContactActionYdirBeforeFlightStuck = ydir.val;
#include "contact_action_flight_stuck.inc"
}
#undef ObjectActionTumble
#undef DFA_FLIGHT

inline bool ObjectActionTumble(C4Object *, int32_t, C4Fixed, C4Fixed) { return false; }
inline bool ObjectActionJump(C4Object *, C4Fixed, C4Fixed, bool) { return false; }

// --- Network rule/goal parameter placement ---------------------------------
// C4Id.h's production representation is a native unsigned long containing the
// four legacy bytes. The focused scaffold keeps only that value contract; all
// list mutation/traversal and scenario/game decisions below execute exact
// bodies mechanically lifted from the pinned C++ source.
using C4ID = unsigned long;
inline constexpr C4ID C4ID_None = 0;

constexpr C4ID C4Id(std::string_view text)
{
    if (text.size() < 4 || text == "NONE") return C4ID_None;
    C4ID id = 0;
    for (std::size_t index = 4; index > 0; --index)
    {
        id <<= 8;
        id |= static_cast<C4ID>(text[index - 1]);
    }
    return id;
}

class C4IDList
{
public:
    C4IDList() = default;
    C4IDList(const C4IDList &) = default;
    C4IDList &operator=(const C4IDList &) = default;

    struct Entry
    {
        C4ID id;
        int32_t count;

        Entry() : id{C4ID_None}, count{0} {}
        Entry(C4ID id, int32_t count) : id{id}, count{count} {}
    };

    // The parsed DefCore appends every entry it reads (C4IDList::CompileFunc,
    // through Entry::CompileFunc), which is the only way the same ID can appear
    // twice — SetIDCount can never produce one. Compiling CompileFunc itself
    // would drag in StdCompiler and the whole group layer, so the oracle
    // constructs the parsed result directly. Nothing under test comes from
    // here: every accessor below is the extracted production code.
    C4IDList(std::initializer_list<Entry> parsed) : content{parsed} {}

    void Clear();
    C4ID GetID(std::size_t index, int32_t *count = nullptr) const;
    int32_t GetIDCount(C4ID id, int32_t zeroDefaultValue = 0) const;
    bool SetIDCount(C4ID id, int32_t count, bool addNewID = false);
    int32_t GetNumberOfIDs() const;

private:
    std::vector<Entry> content;
};

#include "id_list_find.inc"
#include "id_list_clear.inc"
#include "id_list_get_id.inc"
#include "id_list_get_id_count.inc"
#include "id_list_set_id_count.inc"
#include "id_list_get_number_of_ids.inc"

struct C4NameListOracle
{
    void Clear() {}
};

struct C4SRealism
{
    bool ConstructionNeedsMaterial{};
    bool StructuresNeedEnergy{};
};

inline constexpr int32_t C4S_Cooperative = 0;
inline constexpr int32_t C4S_Melee = 1;
inline constexpr int32_t C4S_MeleeTeamwork = 2;
inline constexpr int32_t C4S_KillTheCaptain = 0;
inline constexpr int32_t C4S_CaptureTheFlag = 2;
inline constexpr int32_t C4S_Goldmine = 1;
inline constexpr int32_t C4S_Monsterkill = 2;
inline constexpr int32_t C4S_ValueGain = 3;

class C4SGame
{
public:
    int32_t Mode{C4S_Cooperative};
    int32_t Elimination{1};
    bool EnableRemoveFlag{};
    int32_t ValueGain{};
    C4IDList CreateObjects;
    C4IDList ClearObjects;
    C4NameListOracle ClearMaterial;
    int32_t CooperativeGoal{};
    C4IDList Goals;
    C4IDList Rules;

    void ConvertGoals(C4SRealism &realism);

protected:
    void ClearOldGoals();
};

#include "scenario_convert_goals.inc"
#include "scenario_clear_old_goals.inc"

struct C4Game
{
    C4ObjectListOracle Objects;
    C4MaterialMapOracle Material;
    struct
    {
        C4IDList Rules;
        C4IDList Goals;
    } Parameters;
    std::vector<C4ID> InitCreated;
    int32_t UpdateRulesCalls{};
    struct DigSpawnRecord
    {
        int32_t count{};
        int32_t definition{};
        C4Object *creator{};
        int32_t owner{};
        int32_t x{};
        int32_t y{};
        int32_t rotation{};
    } DigSpawn;

    void ShakeObjects(int32_t tx, int32_t ty, int32_t range, int32_t causedBy);
    void BlastObjects(
        int32_t tx, int32_t ty, int32_t level, C4Object *inobj, int32_t causedBy, C4Object *byObj);
    void InitRules();
    void InitGoals();
    void UpdateRules() { ++UpdateRulesCalls; }

    C4Object *CreateObject(C4ID definition, C4Object *)
    {
        InitCreated.push_back(definition);
        return nullptr;
    }

    C4Object *CreateObject(
        int32_t definition,
        C4Object *creator,
        int32_t owner,
        int32_t x,
        int32_t y,
        int32_t rotation)
    {
        DigSpawn = {
            DigSpawn.count + 1,
            definition,
            creator,
            owner,
            x,
            y,
            rotation,
        };
        return nullptr;
    }
};

#include "game_init_rules.inc"
#include "game_init_goals.inc"

static C4Game DigGameOracle;

// Exact production DigOutMaterialCast body. The scaffold records the
// CreateObject arguments, then the fixture continues on the same Random
// ledger for twenty draws.
#define Game DigGameOracle
#define C4ID_None C4ID_None_Oracle
#include "object_dig_out_material_cast.inc"
#undef C4ID_None
#undef Game

// Exact source text generated from src/C4Game.cpp and src/C4Object.cpp.
#define C4D_Living C4D_Living_Oracle
#include "shake_objects.inc"
#include "object_fling.inc"

#define C4D_Object C4D_Object_Oracle
#define C4D_Vehicle C4D_Vehicle_Oracle
#define DFA_FLOAT DFA_FLOAT_Oracle
#define C4FxCall_EngBlast C4FxCall_EngBlast_Oracle
#define C4FxCall_DmgBlast C4FxCall_DmgBlast_Oracle
#include "blast_objects.inc"
#include "object_blast.inc"
#undef C4FxCall_DmgBlast
#undef C4FxCall_EngBlast
#undef DFA_FLOAT
#undef C4D_Vehicle
#undef C4D_Object
#undef C4D_Living

static C4IDList makeIDList(
    std::initializer_list<std::pair<std::string_view, int32_t>> entries)
{
    C4IDList list;
    for (const auto &[id, count] : entries)
        list.SetIDCount(C4Id(id), count, true);
    return list;
}

static void printC4ID(C4ID id)
{
    char text[5]{};
    for (std::size_t index = 0; index < 4; ++index)
        text[index] = static_cast<char>((id >> (index * 8)) & 0xff);
    printf("\"%s\"", text);
}

static void printC4IDList(const C4IDList &list)
{
    printf("[");
    for (int32_t index = 0; index < list.GetNumberOfIDs(); ++index)
    {
        if (index) printf(",");
        int32_t count{};
        const C4ID id = list.GetID(static_cast<std::size_t>(index), &count);
        printf("{\"id\":");
        printC4ID(id);
        printf(",\"count\":%d}", count);
    }
    printf("]");
}

namespace component_order_oracle
{
struct Case
{
    const char *name;
    std::initializer_list<C4IDList::Entry> initial;
    // An empty id means the row applies no SetIDCount.
    const char *setId;
    int32_t setCount;
};

// `C4IDList::Entry` takes a packed C4ID, so the rows name their IDs as text and
// pack them here.
static C4IDList::Entry entry(std::string_view id, int32_t count)
{
    return C4IDList::Entry{C4Id(id), count};
}

void printCases()
{
    const Case cases[] = {
        // The shipped Bazooka DefCore: Components=METL=2;KLAS=1;ENAP=1;ENAP=1.
        // A map keyed by ID reports three entries where GetNumberOfIDs says
        // four, and ComponentConGain/Cutoff index this list by position.
        {"bazooka_defcore",
         {entry("METL", 2), entry("KLAS", 1), entry("ENAP", 1), entry("ENAP", 1)},
         "", 0},
        // The case a map cannot hold at all: it keeps one of the two counts.
        {"repeat_unequal_counts", {entry("ROCK", 3), entry("ROCK", 7)}, "", 0},
        // findId returns the first match, so a write by ID leaves the later
        // repeat alone.
        {"set_updates_the_first_repeat", {entry("ROCK", 3), entry("ROCK", 7)}, "ROCK", 9},
        // An absent ID appends, which is what puts new entries at the end
        // rather than in sorted position.
        {"set_appends_when_absent", {entry("METL", 2)}, "WOOD", 5},
        // A zero-count entry stays in the list and keeps its slot; with the
        // default zeroDefVal, GetIDCount answers 0 for it exactly as it does
        // for an ID that is not there at all.
        {"zero_count_entry_is_retained", {entry("ZERO", 0), entry("IROC", 3)}, "", 0},
        // Nothing sorts the list.
        {"insertion_order_is_kept", {entry("ZZZZ", 1), entry("AAAA", 2)}, "", 0},
    };

    printf("\"component_order\":[");
    for (std::size_t index = 0; index < std::size(cases); ++index)
    {
        const Case &test = cases[index];
        C4IDList list{test.initial};

        if (index) printf(",");
        printf("{\"name\":\"%s\",\"initial\":", test.name);
        printC4IDList(list);
        printf(",\"set\":");
        if (*test.setId)
        {
            printf("{\"id\":");
            printC4ID(C4Id(test.setId));
            printf(",\"count\":%d}", test.setCount);
            list.SetIDCount(C4Id(test.setId), test.setCount, true);
        }
        else
        {
            printf("null");
        }

        printf(",\"entries\":");
        printC4IDList(list);
        printf(",\"number_of_ids\":%d,\"lookups\":[", list.GetNumberOfIDs());
        // One lookup per distinct ID, in first-appearance order: that is the
        // order findId resolves in, and a repeat must not produce two rows.
        std::vector<C4ID> seen;
        for (int32_t position = 0; position < list.GetNumberOfIDs(); ++position)
        {
            const C4ID id = list.GetID(static_cast<std::size_t>(position));
            if (std::find(seen.begin(), seen.end(), id) != seen.end()) continue;
            if (!seen.empty()) printf(",");
            seen.push_back(id);
            printf("{\"id\":");
            printC4ID(id);
            printf(",\"count\":%d}", list.GetIDCount(id));
        }
        printf("]}");
    }
    printf("]");
}
} // namespace component_order_oracle

static void printCreatedIDs(
    const std::vector<C4ID> &created,
    std::size_t begin,
    std::size_t end)
{
    printf("[");
    for (std::size_t index = begin; index < end; ++index)
    {
        if (index != begin) printf(",");
        printC4ID(created[index]);
    }
    printf("]");
}

static void printNetworkRuleGoalPlacementCase(
    const char *name,
    const C4IDList &scenarioRules,
    const C4IDList &scenarioGoals,
    const C4IDList &parameterRules,
    const C4IDList &parameterGoals)
{
    C4Game game;
    game.Parameters.Rules = parameterRules;
    game.Parameters.Goals = parameterGoals;
    game.InitRules();
    const std::size_t ruleEnd = game.InitCreated.size();
    game.InitGoals();

    printf("{\"name\":\"%s\",\"scenario_rules\":", name);
    printC4IDList(scenarioRules);
    printf(",\"scenario_goals\":");
    printC4IDList(scenarioGoals);
    printf(",\"parameter_rules\":");
    printC4IDList(parameterRules);
    printf(",\"parameter_goals\":");
    printC4IDList(parameterGoals);
    printf(",\"rule_objects\":");
    printCreatedIDs(game.InitCreated, 0, ruleEnd);
    printf(",\"goal_objects\":");
    printCreatedIDs(game.InitCreated, ruleEnd, game.InitCreated.size());
    printf(",\"update_rules_calls\":%d}", game.UpdateRulesCalls);
}

static void printNetworkRuleGoalPlacementCases()
{
    // EkeReloaded.c4f/InterplanetaryCivilwar.c4f/
    // HarpoonRace.c4s/Scenario.txt authors RVLR=1 and RACE=1 while omitting
    // StructNeedEnergy. C4Scenario.cpp:233 defaults that field true, and the
    // exact ConvertGoals body (:506-556) appends ENRG before Parameters is
    // sent in JoinData. Exact InitRules/InitGoals then read those parameter
    // lists at C4Game.cpp:4056-4076.
    C4SGame harpoonRace;
    harpoonRace.Rules = makeIDList({{"RVLR", 1}});
    harpoonRace.Goals = makeIDList({{"RACE", 1}});
    const C4IDList rawHarpoonRules = harpoonRace.Rules;
    const C4IDList rawHarpoonGoals = harpoonRace.Goals;
    C4SRealism harpoonRealism{
        .ConstructionNeedsMaterial = false,
        .StructuresNeedEnergy = true,
    };
    harpoonRace.ConvertGoals(harpoonRealism);

    printf("\"network_rule_goal_placement\":[");
    printNetworkRuleGoalPlacementCase(
        "harpoonrace_join_data",
        rawHarpoonRules,
        rawHarpoonGoals,
        harpoonRace.Rules,
        harpoonRace.Goals);
    printf(",");

    // A second source-selection/count edge makes the differential capable of
    // catching a client that rereads Scenario.txt: rules place max(count, 1),
    // goals place exactly count, and neither local list may leak through.
    printNetworkRuleGoalPlacementCase(
        "authoritative_count_edges",
        makeIDList({{"RVLR", 7}}),
        makeIDList({{"RACE", 7}}),
        makeIDList({{"RVLR", 0}, {"ENRG", 2}}),
        makeIDList({{"RACE", 0}}));
    printf("]");
}

namespace player_join_capacity_oracle
{
enum class C4ResStrTableKey
{
    IDS_PRC_TOOMANYPLRS,
};

struct C4GameParameters
{
    int32_t MaxPlayers{};
};

struct C4Game
{
    C4GameParameters Parameters;
};

static C4Game Game;
static std::optional<int32_t> RejectedMaximum;
static int32_t RejectionLogCalls{};

static void Log(C4ResStrTableKey key, int32_t maximum)
{
    assert(key == C4ResStrTableKey::IDS_PRC_TOOMANYPLRS);
    RejectedMaximum = maximum;
    ++RejectionLogCalls;
}

struct C4Player
{
    std::string Name;
    C4Player *Next{};
};

class C4PlayerList
{
public:
    C4Player *First{};

    int GetCount() const;

    void Seed(std::initializer_list<const char *> names)
    {
        for (const char *name : names) Append(name);
    }

    C4Player *Join(const char *name)
    {
#include "player_join_capacity.inc"
        return Append(name);
    }

    std::vector<std::string> Names() const
    {
        std::vector<std::string> names;
        for (C4Player *player = First; player; player = player->Next)
            names.push_back(player->Name);
        return names;
    }

private:
    std::array<C4Player, 3> Storage{};
    std::size_t Used{};

    C4Player *Append(const char *name)
    {
        assert(Used < Storage.size());
        C4Player *player = &Storage[Used++];
        player->Name = name;
        player->Next = nullptr;
        C4Player *last = First;
        for (; last && last->Next; last = last->Next);
        if (last) last->Next = player; else First = player;
        return player;
    }
};

#include "player_list_get_count.inc"

static void printNames(const std::vector<std::string> &names)
{
    printf("[");
    for (std::size_t index = 0; index < names.size(); ++index)
    {
        if (index) printf(",");
        printf("\"%s\"", names[index].c_str());
    }
    printf("]");
}

static void printCases()
{
    struct AdmissionCase
    {
        const char *name;
        int32_t maximum;
        std::initializer_list<const char *> initialNames;
        const char *joiningName;
    };
    const AdmissionCase cases[] = {
        {"zero_rejects_empty", 0, {}, "Zero"},
        {"below_limit_accepts", 2, {"Ada"}, "Bert"},
        {"at_limit_rejects", 2, {"Ada", "Bert"}, "Cara"},
    };

    printf("\"player_join_capacity\":[");
    for (std::size_t index = 0; index < std::size(cases); ++index)
    {
        const AdmissionCase &test = cases[index];
        C4PlayerList players;
        players.Seed(test.initialNames);
        Game.Parameters.MaxPlayers = test.maximum;
        RejectedMaximum.reset();
        RejectionLogCalls = 0;
        const auto namesBefore = players.Names();
        const int32_t countBefore = players.GetCount();
        const bool accepted = players.Join(test.joiningName) != nullptr;
        const auto namesAfter = players.Names();
        assert(RejectionLogCalls == (accepted ? 0 : 1));
        assert(RejectedMaximum == (accepted
            ? std::optional<int32_t>{}
            : std::optional<int32_t>{test.maximum}));

        if (index) printf(",");
        printf("{\"name\":\"%s\",\"max_players\":%d,\"joining_name\":\"%s\","
               "\"count_before\":%d,\"names_before\":",
               test.name, test.maximum, test.joiningName, countBefore);
        printNames(namesBefore);
        printf(",\"accepted\":%s,\"count_after\":%d,\"names_after\":",
               accepted ? "true" : "false", players.GetCount());
        printNames(namesAfter);
        printf("}");
    }
    printf("]");
}
} // namespace player_join_capacity_oracle

namespace savegame_matching_oracle
{
// C4PlayerInfo.h:332. The order is what RestoreSavegameInfos iterates over
// (`for (int eMatchingLevel = 0; eMatchingLevel <= PML_Any; ++eMatchingLevel)`).
enum MatchingLevel { PML_PlrFileName = 0, PML_PlrName, PML_PrefColor, PML_Any };

template <class T> inline constexpr bool Inside(T ival, T lbound, T rbound)
{
    return ival >= lbound && ival <= rbound;
}

constexpr std::size_t SizeMax = static_cast<std::size_t>(-1);

#include "char_capital.inc"
// The default length lives on the declaration, not the definition
// (C4Strings.h:55), and the extracted switch calls the two-argument form.
bool SEqualNoCase(const char *szStr1, const char *szStr2, size_t iLen = SizeMax);
#include "sequal_no_case.inc"

// StdFile.h:41-49 verbatim. GetFilename therefore splits on both slashes on
// Windows but only on '/' elsewhere — which is why no case below uses a
// backslash path, so the recorded values do not depend on the recording host.
#ifdef _WIN32
#define DirectorySeparator '\\'
#else
#define DirectorySeparator '/'
#endif
#include "get_filename.inc"

static const char *GetFilename(const char *szPath)
{
    return GetFilename(const_cast<char *>(szPath));
}

// The accessors the extracted switch reads, plus the two IDs the association
// passes carry: the player's own and the savegame player it took over.
class C4PlayerInfo
{
public:
    const char *Filename{};
    const char *Name{};
    uint32_t OriginalColor{};
    int32_t ID{};
    int32_t AssociatedSavegamePlayer{};

    const char *GetFilename() const { return Filename; }
    const char *GetName() const { return Name; }
    uint32_t GetOriginalColor() const { return OriginalColor; }
    int32_t GetID() const { return ID; }
    int32_t GetAssociatedSavegamePlayerID() const { return AssociatedSavegamePlayer; }
    void SetAssociatedSavegamePlayer(int32_t id) { AssociatedSavegamePlayer = id; }
};

// The extracted switch verbatim, with its `return pInfo` reaching this
// function's result. A level that declines falls out to nullptr.
static C4PlayerInfo *matchAtLevel(const C4PlayerInfo *pMatchInfo, C4PlayerInfo *pInfo, int iMatchLvl)
{
#include "savegame_matching_switch.inc"
    return nullptr;
}

static void printLatin1Bytes(const char *text)
{
    printf("[");
    for (const unsigned char *cursor = reinterpret_cast<const unsigned char *>(text); *cursor; ++cursor)
    {
        if (cursor != reinterpret_cast<const unsigned char *>(text)) printf(",");
        printf("%u", static_cast<unsigned>(*cursor));
    }
    printf("]");
}

void printCases()
{
    struct MatchCase
    {
        const char *name;
        const char *currentFilename;
        const char *currentName;
        uint32_t currentColor;
        const char *savedFilename;
        const char *savedName;
        uint32_t savedColor;
    };

    // Deliberately no backslash paths: GetFilename splits on
    // DirectorySeparator, which is '\\' only on Windows (StdFile.h:41-49), so a
    // backslash case would record a host-specific expectation into a
    // cross-platform gate.
    const MatchCase cases[] = {
        {"file_and_name", "Players/Ada.c4p", "Ada", 0xff0000, "Save/Ada.c4p", "Ada", 0x00ff00},
        {"same_file_other_name", "Players/Ada.c4p", "Ada", 0xff0000, "Save/Ada.c4p", "Bert", 0x00ff00},
        {"other_file_same_name", "Players/Ada.c4p", "Ada", 0xff0000, "Save/Zoe.c4p", "Ada", 0x00ff00},
        {"name_case_insensitive", "Players/Ada.c4p", "aDa", 0xff0000, "Save/Zoe.c4p", "AdA", 0x00ff00},
        {"name_umlaut_fold", "Players/A.c4p", "J\xfcrgen", 0xff0000, "Save/B.c4p", "J\xdcRGEN", 0x00ff00},
        {"color_only", "Players/Ada.c4p", "Ada", 0x123456, "Save/Zoe.c4p", "Bert", 0x123456},
        {"nothing_in_common", "Players/Ada.c4p", "Ada", 0xff0000, "Save/Zoe.c4p", "Bert", 0x00ff00},
        {"empty_current_filename", "", "Ada", 0xff0000, "Save/Ada.c4p", "Ada", 0x00ff00},
        {"empty_saved_filename", "Players/Ada.c4p", "Ada", 0xff0000, "", "Ada", 0x00ff00},
    };

    printf("\"savegame_player_matching\":[");
    for (std::size_t index = 0; index < std::size(cases); ++index)
    {
        const MatchCase &test = cases[index];
        C4PlayerInfo current;
        current.Filename = test.currentFilename;
        current.Name = test.currentName;
        current.OriginalColor = test.currentColor;
        C4PlayerInfo saved;
        saved.Filename = test.savedFilename;
        saved.Name = test.savedName;
        saved.OriginalColor = test.savedColor;

        if (index) printf(",");
        // Player names are legacy Latin-1 byte strings, so they are emitted as
        // byte arrays: the umlaut case carries 0xfc/0xdc, which is not UTF-8 and
        // would make the golden undecodable as a JSON string.
        printf("{\"name\":\"%s\",\"current_filename\":\"%s\",\"current_name\":",
               test.name, test.currentFilename);
        printLatin1Bytes(test.currentName);
        printf(",\"current_color\":%u,\"saved_filename\":\"%s\",\"saved_name\":",
               test.currentColor, test.savedFilename);
        printLatin1Bytes(test.savedName);
        printf(",\"saved_color\":%u,\"matches\":[", test.savedColor);
        for (int level = PML_PlrFileName; level <= PML_Any; ++level)
        {
            if (level) printf(",");
            printf("%s", matchAtLevel(&current, &saved, level) ? "true" : "false");
        }
        printf("]}");
    }
    printf("]");
}

// C4PlayerInfo.cpp:1373-1391, with FindSavegameResumePlayerInfo's search
// (:1094-1121) inlined over one client's player list.
//
// The eligibility test is the production one at :1101: a savegame player is a
// candidate only while no joining player carries its ID *and* none is already
// associated with it. Both halves matter — the first is what stops a savegame
// player being taken over by a join that already holds that ID.
static bool eligible(const std::vector<C4PlayerInfo> &joining, const C4PlayerInfo &candidate)
{
    return std::none_of(joining.begin(), joining.end(), [&candidate](const C4PlayerInfo &player)
    {
        return player.GetID() == candidate.GetID()
            || player.GetAssociatedSavegamePlayerID() == candidate.GetID();
    });
}

struct WildTakeover
{
    std::size_t participant;
    int32_t savegamePlayer;
};

// The pass loop itself. Every level runs over every still-unassociated joining
// player before the next level starts, which is what makes an exact file+name
// match claim its savegame player before any colour-only match can.
static std::vector<WildTakeover> associate(
    std::vector<C4PlayerInfo> &joining,
    std::vector<C4PlayerInfo> &savegamePlayers)
{
    std::vector<WildTakeover> wild;
    for (int level = PML_PlrFileName; level <= PML_Any; ++level)
        for (std::size_t index = 0; index < joining.size(); ++index)
        {
            if (joining[index].GetAssociatedSavegamePlayerID()) continue;
            for (C4PlayerInfo &candidate : savegamePlayers)
            {
                if (!eligible(joining, candidate)) continue;
                if (!matchAtLevel(&joining[index], &candidate, level)) continue;
                joining[index].SetAssociatedSavegamePlayer(candidate.GetID());
                if (level > PML_PlrName)
                    wild.push_back({index, candidate.GetID()});
                break;
            }
        }
    return wild;
}

void printAssociationCases()
{
    struct Player
    {
        int32_t id;
        const char *filename;
        const char *name;
        uint32_t color;
    };

    struct AssociationCase
    {
        const char *name;
        std::vector<Player> joining;
        std::vector<Player> saved;
    };

    const AssociationCase cases[] = {
        // The exact match claims its savegame player in the first pass, so the
        // colour-only join cannot take it later even though it would match at
        // PML_PrefColor.
        {"exact_match_claims_before_a_wild_one",
         {{41, "Players/Ada.c4p", "Ada", 0x111111}, {42, "Players/Bert.c4p", "Bert", 0x222222}},
         {{7, "Save/Ada.c4p", "Ada", 0x222222}, {8, "Save/Zoe.c4p", "Zoe", 0x222222}}},
        // Order within a pass: the first accepting savegame player wins, and
        // the second join falls through to the next one.
        {"first_accepting_savegame_player_wins",
         {{41, "Players/A.c4p", "Ada", 0x111111}, {42, "Players/B.c4p", "Ada", 0x111111}},
         {{7, "Save/X.c4p", "Ada", 0x999999}, {8, "Save/Y.c4p", "Ada", 0x999999}}},
        // PML_Any takes anything left, and every association past PML_PlrName
        // is reported as wild.
        {"leftovers_are_taken_by_any_and_reported_wild",
         {{41, "Players/A.c4p", "Ada", 0x111111}, {42, "Players/B.c4p", "Bert", 0x222222}},
         {{7, "Save/X.c4p", "Zoe", 0x222222}, {8, "Save/Y.c4p", "Cid", 0x333333}}},
        // Fewer savegame players than joins: the surplus join stays at 0.
        {"a_surplus_join_stays_unassociated",
         {{41, "Players/A.c4p", "Ada", 0x111111}, {42, "Players/B.c4p", "Bert", 0x222222}},
         {{7, "Save/A.c4p", "Ada", 0x111111}}},
        // The other half of the eligibility test: a savegame player whose ID a
        // joining player already carries is not a candidate at all, so the
        // join that would otherwise match it falls through to the next one.
        {"a_savegame_id_a_join_already_carries_is_skipped",
         {{7, "Players/A.c4p", "Ada", 0x111111}},
         {{7, "Save/A.c4p", "Ada", 0x111111}, {8, "Save/B.c4p", "Ada", 0x111111}}},
        // No savegame players at all: every join is left alone.
        {"no_savegame_players_associates_nothing",
         {{41, "Players/A.c4p", "Ada", 0x111111}},
         {}},
    };

    const auto emit = [](const std::vector<C4PlayerInfo> &players)
    {
        printf("[");
        for (std::size_t index = 0; index < players.size(); ++index)
        {
            if (index) printf(",");
            printf("{\"id\":%d,\"filename\":\"%s\",\"name\":", players[index].ID, players[index].Filename);
            printLatin1Bytes(players[index].Name);
            printf(",\"color\":%u}", players[index].OriginalColor);
        }
        printf("]");
    };

    printf("\"savegame_association\":[");
    for (std::size_t index = 0; index < std::size(cases); ++index)
    {
        const AssociationCase &test = cases[index];
        const auto build = [](const std::vector<Player> &rows)
        {
            std::vector<C4PlayerInfo> players;
            for (const Player &row : rows)
            {
                C4PlayerInfo player;
                player.ID = row.id;
                player.Filename = row.filename;
                player.Name = row.name;
                player.OriginalColor = row.color;
                players.push_back(player);
            }
            return players;
        };
        std::vector<C4PlayerInfo> joining = build(test.joining);
        std::vector<C4PlayerInfo> saved = build(test.saved);

        if (index) printf(",");
        printf("{\"name\":\"%s\",\"participants\":", test.name);
        emit(joining);
        printf(",\"savegame_players\":");
        emit(saved);

        const std::vector<WildTakeover> wild = associate(joining, saved);

        printf(",\"associations\":[");
        for (std::size_t player = 0; player < joining.size(); ++player)
        {
            if (player) printf(",");
            printf("%d", joining[player].GetAssociatedSavegamePlayerID());
        }
        printf("],\"wild\":[");
        for (std::size_t entry = 0; entry < wild.size(); ++entry)
        {
            if (entry) printf(",");
            printf("{\"participant\":%zu,\"savegame_player\":%d}",
                   wild[entry].participant, wild[entry].savegamePlayer);
        }
        printf("]}");
    }
    printf("]");
}
} // namespace savegame_matching_oracle

static void printDigOutMaterialCastCase()
{
    constexpr uint32_t seed = 28;
    constexpr int32_t ranges[] = {100, 6, 1000, 2};

    DigGameOracle = C4Game{};
    DigGameOracle.Material.Num = 1;
    DigGameOracle.Material.Map[0].Dig2Object = 1;
    DigGameOracle.Material.Map[0].Dig2ObjectRatio = 1;

    C4Object object;
    object.x = 2;
    object.y = 2;
    object.Shape.y = 2;
    object.Shape.Hgt = 7;
    object.MaterialContents[0] = 1;

    FixedRandom(seed);
    const int32_t countBefore = RandomCount;
    const uint32_t holdBefore = RandomHold;
    object.DigOutMaterialCast(false);
    const int32_t countAfterCast = RandomCount;
    const uint32_t holdAfterCast = RandomHold;

    printf("\"dig2object_rng\":{\"seed\":%u,"
           "\"object_x\":%d,\"object_y\":%d,\"shape_y\":%d,\"shape_height\":%d,"
           "\"rng_before\":{\"count\":%d,\"hold\":%u},"
           "\"spawn\":{\"count\":%d,\"definition\":%d,\"owner\":%d,"
           "\"x\":%d,\"y\":%d,\"rotation\":%d},"
           "\"rng_after_cast\":{\"count\":%d,\"hold\":%u},\"next\":[",
           seed, object.x, object.y, object.Shape.y, object.Shape.Hgt,
           countBefore, holdBefore, DigGameOracle.DigSpawn.count,
           DigGameOracle.DigSpawn.definition, DigGameOracle.DigSpawn.owner,
           DigGameOracle.DigSpawn.x, DigGameOracle.DigSpawn.y,
           DigGameOracle.DigSpawn.rotation, countAfterCast, holdAfterCast);
    for (int32_t index = 0; index < 20; ++index)
    {
        if (index) printf(",");
        const int32_t range = ranges[index % 4];
        printf("{\"range\":%d,\"value\":%d}", range, Random(range));
    }
    printf("],\"rng_after\":{\"count\":%d,\"hold\":%u}}", RandomCount, RandomHold);
}

namespace blast_objects_oracle
{
struct Row
{
    const char *Name;
    int32_t Status;
    bool Contained;
    int32_t Category;
    int32_t X;
    int32_t Y;
    int32_t ShapeX;
    int32_t ShapeY;
    int32_t Wdt;
    int32_t Hgt;
    int32_t Mass;
    int32_t Alive;
    int32_t Grab;
    int32_t NoHorizontalMove;
    int32_t Procedure;
    int32_t BlastIncinerate;
};

// The blast lands at (50, 50) with level 20. Every row isolates one gate of the
// selection chain.
const Row selection[] = {
    // Straddles the blast: direct hit AND shock wave, and living, so it takes
    // the extra energy/damage pair before the fling.
    {"living_center", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, 50, 50, -4, -8, 8, 16, 100,
     1, 0, 0, -1, 0},
    // Offset but still inside both the widened shape and the level range.
    {"object_offset", 1, false, C4D_Object_Oracle, 58, 62, -4, -8, 8, 16, 50, 0, 0, 0, -1, 0},
    // Neither test passes.
    {"far_out_of_range", 1, false, C4D_Object_Oracle, 200, 200, -4, -8, 8, 16, 50, 0, 0, 0, -1, 0},
    // Contained objects are skipped entirely by the uncontained arm.
    {"contained", 1, true, C4D_Living_Oracle | C4D_Object_Oracle, 50, 50, -4, -8, 8, 16, 100, 1, 0,
     0, -1, 0},
    // Deleted objects never appear.
    {"deleted", 0, false, C4D_Living_Oracle | C4D_Object_Oracle, 50, 50, -4, -8, 8, 16, 100, 1, 0,
     0, -1, 0},
    // A structure is outside the three shock-wave categories, so a direct hit
    // blasts it but no force is applied.
    {"structure", 1, false, 1 << 1, 50, 50, -4, -8, 8, 16, 100, 0, 0, 0, -1, 0},
    // NoHorizontalMove keeps its shock wave off.
    {"no_horizontal_move", 1, false, C4D_Object_Oracle, 58, 50, -4, -8, 8, 16, 50, 0, 0, 1, -1, 0},
    // A vehicle is skipped unless Grab is exactly 1.
    {"vehicle_no_grab", 1, false, C4D_Vehicle_Oracle, 58, 50, -4, -8, 8, 16, 50, 0, 0, 0, -1, 0},
    {"vehicle_grab", 1, false, C4D_Vehicle_Oracle, 58, 50, -4, -8, 8, 16, 50, 0, 1, 0, -1, 0},
    // A floating object is skipped on the same Grab gate.
    {"floating", 1, false, C4D_Object_Oracle, 58, 50, -4, -8, 8, 16, 50, 0, 0, 0, DFA_FLOAT_Oracle,
     0},
    // Mass drives the fling divisor's clamp from both ends: 10 clamps to the
    // lower bound of 4, 5000 to the upper bound (20, or 8 for living).
    {"light_object", 1, false, C4D_Object_Oracle, 58, 50, -4, -8, 8, 16, 10, 0, 0, 0, -1, 0},
    {"heavy_object", 1, false, C4D_Object_Oracle, 58, 50, -4, -8, 8, 16, 5000, 0, 0, 0, -1, 0},
    {"heavy_living", 1, false, C4D_Living_Oracle, 58, 50, -4, -8, 8, 16, 5000, 1, 0, 0, -1, 0},
    // Exactly on the level boundary in x: `<= level` includes it.
    {"boundary_in", 1, false, C4D_Object_Oracle, 70, 50, -4, -8, 8, 16, 50, 0, 0, 0, -1, 0},
    {"boundary_out", 1, false, C4D_Object_Oracle, 71, 50, -4, -8, 8, 16, 50, 0, 0, 0, -1, 0},
};

// Blast's incinerate gate is `Damage >= Def->BlastIncinerate`, read after its
// own DoDamage — so a threshold at or below the level fires on the same call
// that caused the damage, and one above it does not.
//
// These rows are a SEPARATE case because the oracle records the Incinerate call
// instead of starting the real fire effect. The production effect draws from
// the synchronised stream, so the RNG ledger across this case is not
// comparable and is deliberately not emitted; the selection case above is where
// the ledger is pinned.
const Row incinerate[] = {
    {"incinerates", 1, false, C4D_Object_Oracle, 50, 50, -4, -8, 8, 16, 50, 0, 0, 0, -1, 15},
    {"survives_incinerate", 1, false, C4D_Object_Oracle, 50, 50, -4, -8, 8, 16, 50, 0, 0, 0, -1,
     25},
};

static void print(const char *section, const Row *rows, std::size_t rowCount, bool withRng)
{
    std::vector<C4Object> objects(rowCount);
    std::vector<C4Object::DefOracle> defs(rowCount);
    std::vector<C4ObjectLink> links(rowCount);
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        defs[i].Grab = rows[i].Grab;
        defs[i].NoHorizontalMove = rows[i].NoHorizontalMove;
        defs[i].BlastIncinerate = rows[i].BlastIncinerate;
        objects[i].Def = &defs[i];
        objects[i].Status = rows[i].Status;
        objects[i].Category = rows[i].Category;
        objects[i].x = rows[i].X;
        objects[i].y = rows[i].Y;
        objects[i].Shape.x = rows[i].ShapeX;
        objects[i].Shape.y = rows[i].ShapeY;
        objects[i].Shape.Wdt = rows[i].Wdt;
        objects[i].Shape.Hgt = rows[i].Hgt;
        objects[i].Mass = rows[i].Mass;
        objects[i].Alive = rows[i].Alive;
        // OCF_Alive is derived from Alive in the engine, and Fling reads the
        // OCF bit (C4Object.cpp:1642) where Blast reads the field (:1421) — a
        // fixture setting only one of them would take a combination the engine
        // cannot produce.
        if (rows[i].Alive) objects[i].OCF |= OCF_Alive;
        objects[i].Controller = -1;
        if (rows[i].Procedure >= 0)
        {
            objects[i].Action.Act = 0;
            defs[i].ActMap[0].Procedure = rows[i].Procedure;
        }
        if (rows[i].Contained) objects[i].Contained = &objects[0];
    }

    C4Game game;
    C4ObjectLink *tail = nullptr;
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        links[i].Obj = &objects[i];
        if (tail) tail->Next = &links[i]; else game.Objects.First = &links[i];
        tail = &links[i];
    }

    FixedRandom(4);
    Randomize3();
    const int32_t countBefore = RandomCount;
    const uint32_t holdBefore = RandomHold;
    const int32_t rnd3Before = FRndPtr3;
    game.BlastObjects(50, 50, 20, nullptr, 7, nullptr);

    printf("\"%s\":{\"seed\":4,\"x\":50,\"y\":50,\"level\":20,\"caused_by\":7,", section);
    if (withRng)
        printf("\"rng_before\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},"
               "\"rng_after\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},",
               countBefore, holdBefore, rnd3Before, RandomCount, RandomHold, FRndPtr3);
    printf("\"objects\":[");
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        if (i) printf(",");
        printf("{\"name\":\"%s\",\"status\":%d,\"contained\":%d,\"category\":%d,"
               "\"x\":%d,\"y\":%d,\"shape_x\":%d,\"shape_y\":%d,\"wdt\":%d,\"hgt\":%d,"
               "\"mass\":%d,\"alive\":%d,\"grab\":%d,\"no_horizontal_move\":%d,"
               "\"procedure\":%d,\"blast_incinerate\":%d,\"damage_calls\":%d,"
               "\"damage_sum\":%d,\"energy_calls\":%d,\"energy_sum\":%d,"
               "\"incinerate_calls\":%d,\"xdir_after\":%d,\"ydir_after\":%d,"
               "\"mobile_after\":%d,\"controller_after\":%d}",
               rows[i].Name, rows[i].Status, rows[i].Contained ? 1 : 0, rows[i].Category,
               rows[i].X, rows[i].Y, rows[i].ShapeX, rows[i].ShapeY, rows[i].Wdt, rows[i].Hgt,
               rows[i].Mass, rows[i].Alive, rows[i].Grab, rows[i].NoHorizontalMove,
               rows[i].Procedure, rows[i].BlastIncinerate, objects[i].DamageCalls,
               objects[i].DamageSum, objects[i].EnergyCalls, objects[i].EnergySum,
               objects[i].IncinerateCalls, objects[i].xdir.val, objects[i].ydir.val,
               objects[i].Mobile, objects[i].Controller);
    }
    printf("]}");
}

static void printCases()
{
    print("blast_objects", selection, sizeof(selection) / sizeof(selection[0]), true);
    printf(",\n");
    print("blast_incinerate_gate", incinerate, sizeof(incinerate) / sizeof(incinerate[0]), false);
}
} // namespace blast_objects_oracle

static void printShakeObjectsCase()
{
    struct Row
    {
        const char *Name;
        int32_t Status;
        bool Contained;
        int32_t Category;
        int32_t X;
        int32_t Y;
        uint8_t Attach;
        int32_t AttachMat;
        uint32_t Ocf;
    };
    const Row rows[] = {
        {"deleted", 0, false, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, 0, 0},
        {"boundary_unattached", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, -10, 30, 0, 0, 0},
        {"contained", 1, true, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, 0, 0},
        {"vehicle", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, 1, 0},
        {"caller", 1, false, C4D_Object_Oracle, 10, 10, CNAT_Bottom, 0, 0},
        {"inactive_attached", 2, false, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, 0, 0},
        {"out_of_range", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, 31, 10, CNAT_Bottom, 0, 0},
        {"attached_gate_rejected", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, 0, 0},
        {"attached_mnone", 1, false, C4D_Living_Oracle | C4D_Object_Oracle, 10, 10, CNAT_Bottom, MNone_Oracle, 0},
    };
    constexpr std::size_t rowCount = sizeof(rows) / sizeof(rows[0]);
    C4Object objects[rowCount]{};
    C4ObjectLink links[rowCount]{};
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        objects[i].Status = rows[i].Status;
        objects[i].Category = rows[i].Category;
        objects[i].x = rows[i].X;
        objects[i].y = rows[i].Y;
        objects[i].Action.t_attach = rows[i].Attach;
        objects[i].Shape.AttachMat = rows[i].AttachMat;
        objects[i].OCF = rows[i].Ocf;
        objects[i].Controller = -1;
        objects[i].xdir.val = 1000 + static_cast<int32_t>(i);
        objects[i].ydir.val = -(2000 + static_cast<int32_t>(i));
    }
    objects[2].Contained = &objects[4];

    C4Game game;
    C4ObjectLink *tail = nullptr;
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        // C4GameObjects::Add places C4OS_INACTIVE objects solely in
        // InactiveObjects, outside the Game.Objects list ShakeObjects walks.
        if (objects[i].Status == 2) continue;
        links[i].Obj = &objects[i];
        if (tail) tail->Next = &links[i]; else game.Objects.First = &links[i];
        tail = &links[i];
    }
    FixedRandom(2);
    Randomize3();
    const int32_t countBefore = RandomCount;
    const uint32_t holdBefore = RandomHold;
    const int32_t rnd3Before = FRndPtr3;
    game.ShakeObjects(10, 10, 20, 7);

    printf("\"shake_objects\":{\"seed\":2,\"x\":10,\"y\":10,\"range\":20,"
           "\"caused_by\":7,\"rng_before\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},"
           "\"rng_after\":{\"count\":%d,\"hold\":%u,\"rnd3_ptr\":%d},\"objects\":[",
           countBefore, holdBefore, rnd3Before, RandomCount, RandomHold, FRndPtr3);
    for (std::size_t i = 0; i < rowCount; ++i)
    {
        if (i) printf(",");
        printf("{\"name\":\"%s\",\"status\":%d,\"contained\":%d,\"category\":%d,"
               "\"x\":%d,\"y\":%d,\"t_attach_before\":%u,\"attach_mat\":%d,\"ocf\":%u,"
               "\"xdir_before\":%d,\"ydir_before\":%d,\"xdir_after\":%d,\"ydir_after\":%d,"
               "\"t_attach_after\":%u,\"mobile_after\":%d,\"controller_after\":%d}",
               rows[i].Name, rows[i].Status, rows[i].Contained ? 1 : 0, rows[i].Category,
               rows[i].X, rows[i].Y, static_cast<unsigned int>(rows[i].Attach), rows[i].AttachMat,
               rows[i].Ocf, 1000 + static_cast<int32_t>(i), -(2000 + static_cast<int32_t>(i)),
               objects[i].xdir.val, objects[i].ydir.val,
               static_cast<unsigned int>(objects[i].Action.t_attach), objects[i].Mobile,
               objects[i].Controller);
    }
    printf("]}");
}

static void printContactActionBottomFlightCases()
{
    struct Row
    {
        const char *Name;
        uint32_t Ocf;
        int32_t Disabled;
    };
    const Row rows[] = {
        {"low_speed_enabled", 0, 0},
        {"low_speed_disabled", 0, 1},
        {"hit_speed4_enabled", OCF_HitSpeed4, 0},
    };

    printf("\"contact_action_bottom_flight\":[");
    for (std::size_t index = 0; index < sizeof(rows) / sizeof(rows[0]); ++index)
    {
        C4Object object;
        object.Action.Dir = 1;
        object.OCF = rows[index].Ocf;
        object.xdir.val = 32768;
        object.ydir.val = 6553;
        object.ContactActionBottomFlight(rows[index].Disabled);
        if (index) printf(",");
        printf("{\"name\":\"%s\",\"ocf\":%u,\"disabled\":%d,"
               "\"xdir_before\":32768,\"ydir_before\":6553,"
               "\"action_after\":%d,\"direction_after\":%d,"
               "\"xdir_after\":%d,\"ydir_after\":%d}",
               rows[index].Name, rows[index].Ocf, rows[index].Disabled,
               object.ContactAction, object.Action.Dir, object.xdir.val, object.ydir.val);
    }
    printf("]");
}

static void printContactActionTopSideFlightCases()
{
    struct Row
    {
        const char *Name;
        uint32_t Contact;
        int32_t Disabled;
        int32_t CanScale;
        int32_t CanHangle;
    };
    const Row rows[] = {
        {"top_enabled", CNAT_Top, 0, 0, 1},
        {"top_disabled", CNAT_Top, 1, 0, 1},
        {"left_enabled", CNAT_Left, 0, 1, 0},
        {"left_disabled", CNAT_Left, 1, 1, 0},
        {"right_enabled", CNAT_Right, 0, 1, 0},
        {"right_disabled", CNAT_Right, 1, 1, 0},
    };

    printf("\"contact_action_top_side_flight\":[");
    for (std::size_t index = 0; index < sizeof(rows) / sizeof(rows[0]); ++index)
    {
        C4Object object;
        object.Action.Dir = DIR_Right;
        object.OCF = 0;
        object.x = 10;
        object.y = 10;
        object.xdir.val = 32768;
        object.ydir.val = 6553;
        C4PhysicalInfo physical{rows[index].CanScale, rows[index].CanHangle};
        switch (rows[index].Contact)
        {
        case CNAT_Top: object.ContactActionTopFlight(rows[index].Disabled, &physical); break;
        case CNAT_Left: object.ContactActionLeftFlight(rows[index].Disabled, &physical); break;
        case CNAT_Right: object.ContactActionRightFlight(rows[index].Disabled, &physical); break;
        }
        if (index) printf(",");
        printf("{\"name\":\"%s\",\"contact\":%u,\"ocf\":0,\"disabled\":%d,"
               "\"can_scale\":%d,\"can_hangle\":%d,"
               "\"x_before\":10,\"y_before\":10,"
               "\"xdir_before\":32768,\"ydir_before\":6553,"
               "\"action_after\":%d,\"direction_after\":%d,"
               "\"xdir_before_flight_stuck\":%d,\"ydir_before_flight_stuck\":%d,"
               "\"x_after\":%d,\"y_after\":%d,"
               "\"xdir_after\":%d,\"ydir_after\":%d}",
               rows[index].Name, rows[index].Contact, rows[index].Disabled,
               rows[index].CanScale, rows[index].CanHangle,
               object.ContactAction, object.Action.Dir,
               object.ContactActionXdirBeforeFlightStuck,
               object.ContactActionYdirBeforeFlightStuck, object.x, object.y,
               object.xdir.val, object.ydir.val);
    }
    printf("]");
}

// --- tiny JSON helpers (avoid a JSON dependency in the oracle) ---------------
static bool g_first_in_array = true;
static void arr_begin(const char *name) { printf("\"%s\":[", name); g_first_in_array = true; }
static void arr_end() { printf("]"); }
static void sep() { if (!g_first_in_array) printf(","); g_first_in_array = false; }

static void consume_corrode_effect_rng()
{
    if (!Random(5)) Random(3);
    if (!Random(20)) { /* sound effect decision only; payload has no RNG */ }
}

struct KillerOracleObject
{
    int32_t LastEnergyLossCausePlayer{NO_OWNER};
};

static void printScriptKillerCases()
{
    KillerOracleObject self, other;
    const auto validPlayer = [](int32_t player) { return player == 1; };

    const int32_t initial = C4ScriptKiller::Get(&self, static_cast<KillerOracleObject *>(nullptr));
    const bool setSelf = C4ScriptKiller::Set(1, &self, static_cast<KillerOracleObject *>(nullptr), validPlayer);
    const int32_t readSelf = C4ScriptKiller::Get(&self, static_cast<KillerOracleObject *>(nullptr));
    const bool setInvalid = C4ScriptKiller::Set(9, &self, static_cast<KillerOracleObject *>(nullptr), validPlayer);
    const int32_t afterInvalid = C4ScriptKiller::Get(&self, static_cast<KillerOracleObject *>(nullptr));
    const bool clearSelf = C4ScriptKiller::Set(NO_OWNER, &self, static_cast<KillerOracleObject *>(nullptr), validPlayer);
    const int32_t readCleared = C4ScriptKiller::Get(&self, static_cast<KillerOracleObject *>(nullptr));
    const bool setForeign = C4ScriptKiller::Set(1, &self, &other, validPlayer);
    const int32_t readForeign = C4ScriptKiller::Get(&self, &other);
    // An arrow engine-function call runs with the target as cthr->Obj and no
    // explicit pObj argument, so this is its exact fallback-target shape.
    const bool arrowClear = C4ScriptKiller::Set(NO_OWNER, &other, static_cast<KillerOracleObject *>(nullptr), validPlayer);
    const int32_t arrowRead = C4ScriptKiller::Get(&other, static_cast<KillerOracleObject *>(nullptr));
    const int32_t getNoContext = C4ScriptKiller::Get<KillerOracleObject>(nullptr, nullptr);
    const bool setNoContext = C4ScriptKiller::Set<KillerOracleObject>(1, nullptr, nullptr, validPlayer);

    printf("\"script_killer\":{\"initial\":%d,\"set_self\":%d,\"read_self\":%d,"
           "\"set_invalid\":%d,\"after_invalid\":%d,\"clear_self\":%d,"
           "\"read_cleared\":%d,\"set_foreign\":%d,\"read_foreign\":%d,"
           "\"arrow_clear\":%d,\"arrow_read\":%d,\"self_final\":%d,"
           "\"foreign_final\":%d,\"get_no_context\":%d,\"set_no_context\":%d}",
           initial, setSelf ? 1 : 0, readSelf, setInvalid ? 1 : 0, afterInvalid,
           clearSelf ? 1 : 0, readCleared, setForeign ? 1 : 0, readForeign,
           arrowClear ? 1 : 0, arrowRead, self.LastEnergyLossCausePlayer,
           other.LastEnergyLossCausePlayer, getNoContext, setNoContext ? 1 : 0);
}

// --- FnEval -> DirectExec receiver and temporary scope -----------------------
// `gen_golden.sh` lifts FnEval in full (C4Script.cpp:4501-4513) and the
// decisive DirectExec child-scope block (C4AulExec.cpp:1674-1683). These
// minimal surrounding types make receiver choice, object Def/LocalNamed copy,
// parent registration, eval arguments, and strictness observable without
// transcribing either production decision.
enum class EvalScriptStrict : uint8_t
{
    NONSTRICT = 0,
    STRICT1 = 1,
    STRICT2 = 2,
    STRICT3 = 3,
    MAXSTRICT = STRICT3,
};

using EvalValue = int32_t;

struct EvalScriptEngine
{
};

struct EvalLocalNamed
{
    int32_t Identity{};
};

struct EvalDefinition;
struct EvalObject;

struct EvalScript
{
    EvalScriptStrict Strict{EvalScriptStrict::MAXSTRICT};
    EvalDefinition *Def{};
    EvalLocalNamed LocalNamed{};
    EvalScriptEngine *Engine{};
    EvalScript *Parent{};
    int32_t Identity{};
    std::string_view ExpectedSource;
    bool LastCalled{};
    bool LastScopeValid{};
    EvalScriptStrict LastStrict{EvalScriptStrict::MAXSTRICT};

    void Reg2List(EvalScriptEngine *engine, EvalScript *parent)
    {
        Engine = engine;
        Parent = parent;
    }

    EvalValue DirectExec(
        EvalObject *pObj,
        const char *szScript,
        const char *szContext,
        bool fPassErrors,
        EvalScriptStrict strict);
};

struct EvalDefinition
{
    EvalScript Script;
};

struct EvalObject
{
    EvalDefinition *Def{};
};

EvalValue EvalScript::DirectExec(
    EvalObject *pObj,
    const char *szScript,
    const char *szContext,
    bool fPassErrors,
    EvalScriptStrict strict)
{
    EvalScript temporary;
    EvalScript *pScript = &temporary;

#include "script_direct_exec_scope.inc"

    LastCalled = true;
    LastStrict = strict;
    const bool objectScopeValid = pObj
        ? this == &pObj->Def->Script
            && pScript->Def == pObj->Def
            && pScript->LocalNamed.Identity == pObj->Def->Script.LocalNamed.Identity
        : pScript->Def == nullptr && pScript->LocalNamed.Identity == 0;
    LastScopeValid =
        objectScopeValid
        && pScript->Engine == Engine
        && pScript->Parent == this
        && std::string_view{szScript ? szScript : ""} == ExpectedSource
        && std::string_view{szContext ? szContext : ""} == "eval"
        && fPassErrors;
    return LastScopeValid ? Identity : -Identity;
}

struct EvalFunction
{
    EvalScript *pOrgScript{};
};

struct EvalContext
{
    EvalContext *Caller{};
    EvalFunction *Func{};
    EvalObject *Obj{};
    EvalDefinition *Def{};
};

struct EvalString
{
    const char *Data{};
};

static const char *EvalStringPar(EvalString *string)
{
    return string && string->Data ? string->Data : "";
}

struct EvalGame
{
    EvalScript Script;
};

static EvalGame EvalGameOracle;

#define C4Value EvalValue
#define C4AulContext EvalContext
#define C4String EvalString
#define C4AulScriptStrict EvalScriptStrict
#define FnStringPar EvalStringPar
#define FnEval EvalFnOracle
#define Game EvalGameOracle
#include "script_fn_eval.inc"
#undef Game
#undef FnEval
#undef FnStringPar
#undef C4AulScriptStrict
#undef C4String
#undef C4AulContext
#undef C4Value

static void printEvalDirectExecContextCases()
{
    struct Case
    {
        const char *Name;
        bool HasObject;
        bool HasDefinition;
        EvalScriptStrict CallerStrict;
        const char *Source;
        int32_t ExpectedReceiver;
    };
    const Case cases[] = {
        {
            "object_definition",
            true,
            true,
            EvalScriptStrict::STRICT2,
            "Explode(power)",
            1,
        },
        {
            "definition_only",
            false,
            true,
            EvalScriptStrict::STRICT1,
            "DefinitionHelper()",
            2,
        },
        {
            "game_script",
            false,
            false,
            EvalScriptStrict::STRICT3,
            "ScenarioHelper()",
            3,
        },
    };

    printf("\"eval_direct_exec_context\":[");
    for (std::size_t index = 0; index < sizeof(cases) / sizeof(cases[0]); ++index)
    {
        EvalScriptEngine engine;
        EvalDefinition objectDefinition;
        objectDefinition.Script.Engine = &engine;
        objectDefinition.Script.Identity = 51;
        objectDefinition.Script.LocalNamed.Identity = 501;
        objectDefinition.Script.ExpectedSource = "Explode(power)";
        EvalObject object{&objectDefinition};

        EvalDefinition definition;
        definition.Script.Engine = &engine;
        definition.Script.Identity = 62;
        definition.Script.LocalNamed.Identity = 602;
        definition.Script.ExpectedSource = "DefinitionHelper()";

        EvalGameOracle = EvalGame{};
        EvalGameOracle.Script.Engine = &engine;
        EvalGameOracle.Script.Identity = 73;
        EvalGameOracle.Script.LocalNamed.Identity = 703;
        EvalGameOracle.Script.ExpectedSource = "ScenarioHelper()";

        EvalScript callerOrigin;
        callerOrigin.Strict = cases[index].CallerStrict;
        EvalFunction callerFunction{&callerOrigin};
        EvalContext caller;
        caller.Func = &callerFunction;
        EvalContext context;
        context.Caller = &caller;
        context.Obj = cases[index].HasObject ? &object : nullptr;
        context.Def = cases[index].HasDefinition ? &definition : nullptr;
        EvalString source{cases[index].Source};

        const EvalValue result = EvalFnOracle(&context, &source);
        EvalScript *called = objectDefinition.Script.LastCalled
            ? &objectDefinition.Script
            : definition.Script.LastCalled ? &definition.Script : &EvalGameOracle.Script;
        const int32_t receiver = called == &objectDefinition.Script
            ? 1
            : called == &definition.Script ? 2 : 3;

        if (index) printf(",");
        printf(
            "{\"name\":\"%s\",\"has_object\":%d,\"has_definition\":%d,"
            "\"caller_strict\":%u,\"expected_receiver\":%d,\"receiver\":%d,"
            "\"scope_valid\":%d,\"direct_strict\":%u,\"result\":%d}",
            cases[index].Name,
            cases[index].HasObject ? 1 : 0,
            cases[index].HasDefinition ? 1 : 0,
            static_cast<unsigned int>(cases[index].CallerStrict),
            cases[index].ExpectedReceiver,
            receiver,
            called->LastScopeValid ? 1 : 0,
            static_cast<unsigned int>(called->LastStrict),
            result);
    }
    printf("]");
}

static void printLandscapePathCases()
{
    struct PathCase { const char *name; int32_t pixelX, pixelY, density; };
    const PathCase cases[] = {
        {"empty", -1, -1, 0},
        {"right_edge_water", 16, 9, 25},
    };
    printf("\"landscape_path\":[");
    bool first = true;
    for (const auto &pathCase : cases)
    {
        int32_t densities[17 * 15]{};
        if (pathCase.pixelX >= 0 && pathCase.pixelY >= 0)
            densities[pathCase.pixelY * 17 + pathCase.pixelX] = pathCase.density;
        const bool free = C4LandscapePath::IsFree(0, 0, 16, 14, [&densities](int32_t cellX, int32_t cellY)
        {
            if (cellX != 0 || cellY != 0) return false;
            for (const int32_t density : densities)
                if (density != 0) return true;
            return false;
        });
        if (!first) printf(",");
        first = false;
        printf("{\"name\":\"%s\",\"pixel_x\":%d,\"pixel_y\":%d,\"density\":%d,\"free\":%d}",
               pathCase.name, pathCase.pixelX, pathCase.pixelY, pathCase.density, free ? 1 : 0);
    }
    printf("]");
}

static void printActionDirectionCase()
{
    // Minimized from Goldrush frame 170, WIPF #566. WALK steers Right, but
    // residual raw xdir remains negative after WalkAccel. C++ tests that raw
    // sign, calls SetDir(Left), fires Walk.TurnAction and SetAction snaps the
    // fixed accumulator before movement. The old WALK pAction remains the
    // phase source for the rest of ExecAction.
    C4Fixed xdir;
    xdir.val = -52430;
    xdir += itofix(50, 100); // WalkAccel = FIXED100(50)
    const auto update = C4ActionDirection::FromHorizontalVelocity(xdir, 10);
    const int32_t requestedDirection = update.Direction == C4ActionDirection::Horizontal::Left
        ? 0
        : update.Direction == C4ActionDirection::Horizontal::Right ? 1 : -1;
    const int32_t currentDirection = 1;
    const bool runsTurnAction = requestedDirection >= 0
        && C4ActionDirection::RunsTurnAction(currentDirection, requestedDirection, true);

    C4Fixed fixX;
    fixX.val = 35468082;
    int32_t actionTime = 0;
    int32_t actionPhase = 0;
    int32_t actionPhaseDelay = 0;
    bool actionIsTurn = false;

    // Action.Time++ occurs before WALK. SetAction(Turn) then resets time,
    // phase, phase delay and fixed position (C4Object.cpp:4745, 4118-4167).
    actionTime++;
    if (runsTurnAction)
    {
        actionIsTurn = true;
        actionTime = actionPhase = actionPhaseDelay = 0;
        fixX = itofix(541);
    }

    const int32_t fixXAfterSetDir = fixX.val;
    // ExecAction keeps the pre-transition WALK pAction: Delay=2, Step=1.
    actionPhaseDelay += update.PhaseAdvance;
    if (actionPhaseDelay >= 2)
    {
        actionPhaseDelay = 0;
        actionPhase += 1;
    }
    fixX += xdir;

    printf("\"action_direction\":{\"initial_xdir\":-52430,\"steered_xdir\":%d,"
           "\"requested_dir\":%d,\"phase_advance\":%d,\"runs_turn_action\":%d,"
           "\"action_is_turn\":%d,\"direction\":%d,\"command_direction\":3,"
           "\"action_time\":%d,\"action_phase\":%d,\"action_phase_delay\":%d,"
           "\"fix_x_after_set_dir\":%d,\"fix_x_after_move\":%d}",
           xdir.val, requestedDirection, update.PhaseAdvance, runsTurnAction ? 1 : 0,
           actionIsTurn ? 1 : 0, requestedDirection, actionTime, actionPhase,
           actionPhaseDelay, fixXAfterSetDir, fixX.val);
}

// --- DFA_PUSH/PULL/FIGHT direction blocks: exact production text -----------
// gen_golden.sh lifts these blocks from C4Object::ExecAction. The scaffold
// supplies the already-computed raw xdir or target position and records the
// observable SetDir calls without restating the branch conditions.
namespace exec_action_direction_oracle
{
struct TargetState
{
    int32_t x{};
};

struct Actor
{
    struct ActionState
    {
        TargetState *Target{};
        int32_t Dir{DIR_Left};
    } Action;

    C4Fixed xdir{Fix0};
    int32_t x{};
    int32_t iPhaseAdvance{1};
    int32_t SetDirCalls{};
    bool RunsTurnAction{};
    int32_t TurnStartDir{-1};

    void SetDir(int32_t direction)
    {
        ++SetDirCalls;
        if (C4ActionDirection::RunsTurnAction(Action.Dir, direction, true))
        {
            RunsTurnAction = true;
            TurnStartDir = Action.Dir;
        }
        Action.Dir = direction;
    }

    void RunPush()
    {
#include "object_push_direction.inc"
    }

    void RunPull()
    {
#include "object_pull_direction.inc"
    }

    void RunFight()
    {
#include "object_fight_direction.inc"
    }
};

static void printRow(const char *name, const Actor &actor, int32_t actorX,
                     int32_t targetX, int32_t initialDirection)
{
    printf("{\"name\":\"%s\",\"xdir_raw\":%d,\"xdir_pixel\":%d,"
           "\"actor_x\":%d,\"target_x\":%d,\"initial_direction\":%d,"
           "\"set_dir_calls\":%d,"
           "\"runs_turn_action\":%d,\"turn_start_dir\":%d,"
           "\"direction\":%d}",
           name, actor.xdir.val, fixtoi(actor.xdir), actorX, targetX,
           initialDirection, actor.SetDirCalls, actor.RunsTurnAction ? 1 : 0,
           actor.TurnStartDir, actor.Action.Dir);
}

static void printCases()
{
    printf("\"action_push_pull_fight_direction\":[");

    Actor push;
    push.xdir.val = 1;
    push.RunPush();
    printRow("push_positive_subpixel", push, 0, 10, DIR_Left);

    Actor pull;
    pull.xdir.val = 1;
    pull.RunPull();
    printf(",");
    printRow("pull_positive_subpixel", pull, 20, 0, DIR_Left);

    TargetState rightTarget{10};
    Actor fightRight;
    fightRight.xdir.val = -61790;
    fightRight.Action.Target = &rightTarget;
    fightRight.RunFight();
    printf(",");
    printRow("fight_target_right_negative_velocity", fightRight, 0, 10, DIR_Left);

    TargetState equalTarget{0};
    Actor fightEqual;
    fightEqual.xdir.val = -61790;
    fightEqual.Action.Target = &equalTarget;
    fightEqual.Action.Dir = DIR_Right;
    fightEqual.RunFight();
    printf(",");
    printRow("fight_equal_x_negative_velocity", fightEqual, 0, 0, DIR_Right);

    printf("]");
}
} // namespace exec_action_direction_oracle

static void printSwimActionDirectionCase()
{
    // Minimized from Goldrush frame 219, FISH #1343. COMD_Left applies one
    // SwimAccel to a zero xdir. DFA_SWIM then calls SetDir(Left), which fires
    // Swim.TurnAction and SetAction snaps BOTH fixed accumulators to the
    // integer position before movement. ExecAction retains the old Swim action
    // as its phase source, so the newly installed Turn advances to phase 1.
    C4Fixed xdir, ydir;
    xdir.val = 0;
    ydir.val = -6556;
    xdir -= itofix(20, 100); // SwimAccel = FIXED100(20)
    const auto update = C4ActionDirection::FromHorizontalVelocity(xdir, 0);
    const int32_t requestedDirection = update.Direction == C4ActionDirection::Horizontal::Left
        ? 0
        : update.Direction == C4ActionDirection::Horizontal::Right ? 1 : -1;
    const int32_t currentDirection = 1;
    const bool runsTurnAction = requestedDirection >= 0
        && C4ActionDirection::RunsTurnAction(currentDirection, requestedDirection, true);

    C4Fixed fixX, fixY;
    fixX.val = 57212928;
    fixY.val = 28737532;
    int32_t actionTime = 103;
    int32_t actionPhase = 3;
    int32_t actionPhaseDelay = 0;
    bool actionIsTurn = false;

    actionTime++;
    if (runsTurnAction)
    {
        actionIsTurn = true;
        actionTime = actionPhase = actionPhaseDelay = 0;
        fixX = itofix(873);
        fixY = itofix(438);
    }

    const int32_t fixXAfterSetDir = fixX.val;
    const int32_t fixYAfterSetDir = fixY.val;
    // Full Swim physical: lLimit=FIXED100(160), phase advance=16, Delay=1.
    const int32_t phaseAdvance = fixtoi(itofix(160, 100) * 10);
    actionPhaseDelay += phaseAdvance;
    if (actionPhaseDelay >= 1)
    {
        actionPhaseDelay = 0;
        actionPhase += 1;
    }
    fixX += xdir;
    fixY += ydir;

    printf("\"action_swim_direction\":{\"initial_xdir\":0,\"initial_ydir\":-6556,"
           "\"steered_xdir\":%d,\"steered_ydir\":%d,\"requested_dir\":%d,"
           "\"phase_advance\":%d,\"runs_turn_action\":%d,\"action_is_turn\":%d,"
           "\"direction\":%d,\"command_direction\":7,\"action_time\":%d,"
           "\"action_phase\":%d,\"action_phase_delay\":%d,"
           "\"fix_x_after_set_dir\":%d,\"fix_y_after_set_dir\":%d,"
           "\"fix_x_after_move\":%d,\"fix_y_after_move\":%d}",
           xdir.val, ydir.val, requestedDirection, phaseAdvance,
           runsTurnAction ? 1 : 0, actionIsTurn ? 1 : 0, requestedDirection,
           actionTime, actionPhase, actionPhaseDelay, fixXAfterSetDir,
           fixYAfterSetDir, fixX.val, fixY.val);
}

static void printActionCallbackCases()
{
    // Minimized from Goldrush frame 192, WIPF #565. Script SetAction requests
    // Start+Abort; Sit's StartCall must run exactly once, before Walk's
    // optional AbortCall. Natural phase wraps request Start+End in that order.
    struct CallbackCase
    {
        const char *Name;
        bool EndRequested;
        bool AbortRequested;
        bool EndInstalled;
        bool AbortInstalled;
    };
    const CallbackCase cases[] = {
        {"script_start_only", false, true, false, false},
        {"script_start_abort", false, true, false, true},
        {"natural_start_end", true, false, true, false},
    };

    printf("\"action_callbacks\":[");
    bool first = true;
    for (const auto &test : cases)
    {
        int32_t order = 0, startCount = 0, oldCount = 0;
        const bool completed = C4ActionCallbacks::Dispatch(
            true, test.EndRequested, test.AbortRequested,
            false, true, true,
            [&](C4ActionCallbacks::Kind kind)
            {
                switch (kind)
                {
                case C4ActionCallbacks::Kind::Start:
                    order = order * 10 + 1;
                    ++startCount;
                    break;
                case C4ActionCallbacks::Kind::End:
                    if (test.EndInstalled)
                    {
                        order = order * 10 + 2;
                        ++oldCount;
                    }
                    break;
                case C4ActionCallbacks::Kind::Abort:
                    if (test.AbortInstalled)
                    {
                        order = order * 10 + 3;
                        ++oldCount;
                    }
                    break;
                }
                return true;
            });
        if (!first) printf(",");
        first = false;
        printf("{\"name\":\"%s\",\"completed\":%d,\"callback_order\":%d,"
               "\"start_count\":%d,\"old_count\":%d}",
               test.Name, completed ? 1 : 0, order, startCount, oldCount);
    }
    printf("]");
}

// --- DFA_CONNECT missing target: exact production check/removal branch -----
// gen_golden.sh extracts pinned C4Object.cpp:5368-5376 unchanged. This scaffold
// supplies null targets and models only the observable lifecycle it invokes:
// LineBreak(true), then AssignRemoval's Destruction callback, then Status=0.
struct ConnectOracleBool
{
    bool Value;
};

static ConnectOracleBool C4VBool(bool value) { return {value}; }
static constexpr const char *PSF_LineBreak = "~LineBreak";

struct ConnectMissingTargetOracle
{
    struct TargetState
    {
        int32_t Con;
    };

    struct ActionState
    {
        TargetState *Target{};
        TargetState *Target2{};
    } Action;

    struct GeometryShape
    {
        int32_t VtxNum{1};

        bool LineConnect()
        {
#include "shape_line_connect_vertex_guard.inc"
            return true;
        }
    } Shape;

    static constexpr int32_t FullCon = 100000;
    int32_t Status{1};
    int32_t CallbackOrder{};
    int32_t LineBreakCount{};
    int32_t LineBreakArgumentCount{};
    int32_t DestructionCount{};
    bool LineBreakAutomatic{};

    void Call(const char *function, std::initializer_list<ConnectOracleBool> args)
    {
        if (std::string_view(function) != PSF_LineBreak) return;
        CallbackOrder = CallbackOrder * 10 + 1;
        ++LineBreakCount;
        LineBreakArgumentCount = static_cast<int32_t>(args.size());
        LineBreakAutomatic = args.size() == 1 && args.begin()->Value;
    }

    void Call(const char *function)
    {
        if (std::string_view(function) != PSF_LineBreak) return;
        CallbackOrder = CallbackOrder * 10 + 1;
        ++LineBreakCount;
        LineBreakArgumentCount = 0;
        LineBreakAutomatic = false;
    }

    void Destruction()
    {
        CallbackOrder = CallbackOrder * 10 + 2;
        ++DestructionCount;
    }

    void AssignRemoval()
    {
        Destruction();
        Status = 0;
    }

    void ExecuteMissingTarget()
    {
        bool fBroke = false;
#include "object_connect_missing_target.inc"
    }

    void ExecuteGeometryBreak()
    {
        // The mechanically extracted C4Shape.cpp:275 guard supplies the
        // geometry failure; the later C4Object branch is also exact source.
        bool fBroke = !Shape.LineConnect();
#include "object_connect_geometry_break.inc"
    }
};

static void printConnectRemovalState(const char *section,
                                     const ConnectMissingTargetOracle &object)
{
    printf("\"%s\":{\"callback_order\":%d,\"line_break_count\":%d,"
           "\"line_break_argument_count\":%d,\"line_break_automatic\":%d,"
           "\"destruction_count\":%d,\"status\":%d}",
           section, object.CallbackOrder, object.LineBreakCount,
           object.LineBreakArgumentCount,
           object.LineBreakAutomatic ? 1 : 0, object.DestructionCount,
           object.Status);
}

static void printConnectMissingTargetCase()
{
    ConnectMissingTargetOracle object;
    object.ExecuteMissingTarget();
    printConnectRemovalState("connect_missing_target_removal", object);
}

static void printConnectGeometryBreakCase()
{
    ConnectMissingTargetOracle object;
    object.ExecuteGeometryBreak();
    printConnectRemovalState("connect_geometry_break_removal", object);
}

struct SolidMaskOracleBitmap
{
    bool Transparent;

    bool IsPixTransparent(int32_t, int32_t) { return Transparent; }
};

struct SolidMaskOracleGraphics
{
    SolidMaskOracleBitmap *Bitmap;

    SolidMaskOracleBitmap *GetBitmap() { return Bitmap; }
};

struct SolidMaskOracleObject
{
    SolidMaskOracleGraphics *Graphics;

    SolidMaskOracleGraphics *GetGraphics() { return Graphics; }
};

static void printSolidMaskGraphicsCases()
{
    // Minimized from Goldrush frame 184, CTWR #1351. The decisive mask source
    // pixel (219,86) is transparent in Graphics.png and opaque in Graphics2.png.
    // C4SolidMask samples the object's ACTIVE graphics, including variants.
    SolidMaskOracleBitmap baseBitmap{true};
    SolidMaskOracleBitmap variantBitmap{false};
    SolidMaskOracleGraphics baseGraphics{&baseBitmap};
    SolidMaskOracleGraphics variantGraphics{&variantBitmap};
    SolidMaskOracleObject object{&baseGraphics};
    constexpr int32_t sourceX = 219;
    constexpr int32_t sourceY = 86;

    printf("\"solid_mask_graphics\":[");
    const auto printCase = [&](const char *name, int32_t selectedVariant)
    {
        auto *active = C4SolidMaskBitmap::GetActiveBitmap(&object);
        printf("%s{\"name\":\"%s\",\"selected_variant\":%d,\"active_variant\":%d,"
               "\"source_x\":%d,\"source_y\":%d,\"mask_pixel\":%u}",
               selectedVariant ? "," : "", name, selectedVariant,
               active == &variantBitmap ? 1 : 0, sourceX, sourceY,
               static_cast<unsigned int>(C4SolidMaskBitmap::MaskPixel(active, sourceX, sourceY)));
    };
    printCase("base", 0);
    object.Graphics = &variantGraphics;
    printCase("variant_2", 1);
    printf("]");
}

// --- DefCore Scale -> Picture facet: src/C4Def.cpp:745,1341 ------------------
// C4DefCore::Scale is the uint32 percentage (src/C4Def.h:274); C4Def::Scale is
// the float multiplier it becomes (src/C4Def.h:335), shadowing the base member.
// The scaffold reproduces exactly that shadowing so the two lifted production
// statements below compile and resolve their names as they do in the engine.
struct C4DefCore
{
    uint32_t Scale;
};

struct DefPictureScaleOracle : C4DefCore
{
    float Scale;
    C4Rect PictureRect;

    // Production: `Scale = C4DefCore::Scale / 100.0f;`
    void PostLoadScale()
    {
#include "def_scale_from_defcore.inc"
    }

    // Production: the `const auto scaledRect = ...;` statement of
    // C4Def::Picture2Facet. Phase is composed in GAME units, then the whole
    // rect is scaled, so truncation applies to the already-offset x.
    C4Rect Picture2FacetRect(int32_t xPhase) const
    {
#include "def_picture2facet_rect.inc"
        return scaledRect;
    }
};

static void printDefPictureScaleCases()
{
    struct Case
    {
        const char *name;
        uint32_t scalePercent;
        int32_t x, y, wdt, hgt;
        int32_t xPhase;
    };
    // Scale=100 pins the identity path; the rest pin truncation toward zero and
    // the phase-before-scale composition (a phase applied after scaling would
    // give a different x wherever Wdt * scale truncates).
    const Case cases[] = {
        {"unscaled_phase0",      100,  0,  0, 64, 64, 0},
        {"unscaled_phase2",      100,  0,  0, 64, 64, 2},
        {"double_unit",          200,  0,  0,  1,  1, 0},
        {"double_offset_phase3", 200, 10,  4, 16, 20, 3},
        {"triple_phase1",        300,  5,  7,  9, 11, 1},
        {"one_and_a_half",       150,  1,  1,  3,  3, 1},
        {"one_and_a_quarter",    125,  3,  3,  7,  7, 2},
        {"fractional_third",      33, 10, 10, 10, 10, 0},
    };

    printf("\"def_picture_scale\":[");
    bool first = true;
    for (const auto &c : cases)
    {
        DefPictureScaleOracle def{};
        def.C4DefCore::Scale = c.scalePercent;
        def.PostLoadScale();
        def.PictureRect = C4Rect{c.x, c.y, c.wdt, c.hgt};
        const C4Rect r = def.Picture2FacetRect(c.xPhase);
        printf("%s{\"name\":\"%s\",\"scale_percent\":%u,\"scale\":%.9g,"
               "\"picture_x\":%d,\"picture_y\":%d,\"picture_wdt\":%d,\"picture_hgt\":%d,"
               "\"phase\":%d,\"x\":%d,\"y\":%d,\"wdt\":%d,\"hgt\":%d}",
               first ? "" : ",", c.name, static_cast<unsigned int>(c.scalePercent),
               static_cast<double>(def.Scale),
               c.x, c.y, c.wdt, c.hgt, c.xPhase, r.x, r.y, r.Wdt, r.Hgt);
        first = false;
    }
    printf("]");
}

// --- C4Value hash: mirrors src/C4Value.cpp:923-1029 --------------------------
// based on boost container_hash's hashCombine
static constexpr void hashCombine(std::size_t &hash, std::size_t nextHash)
{
    if constexpr (sizeof(std::size_t) == 4)
    {
#define rotateLeft32(x, r) (x << r) | (x >> (32 - r))
        constexpr std::size_t c1 = 0xcc9e2d51;
        constexpr std::size_t c2 = 0x1b873593;

        nextHash *= c1;
        nextHash = rotateLeft32(nextHash, 15);
        nextHash *= c2;

        hash ^= nextHash;
        hash = rotateLeft32(hash, 13);
        hash = hash * 5 + 0xe6546b64;
#undef rotateLeft32
    }
    else if constexpr (sizeof(std::size_t) == 8)
    {
        constexpr std::size_t m = 0xc6a4a7935bd1e995;
        constexpr int r = 47;

        nextHash *= m;
        nextHash ^= nextHash >> r;
        nextHash *= m;

        hash ^= nextHash;
        hash *= m;

        // Completely arbitrary number, to prevent 0's
        // from hashing to 0.
        hash += 0xe6546b64;
    }
    else
    {
        hash ^= nextHash + 0x9e3779b9 + (hash << 6) + (hash >> 2);
    }
}

enum C4V_Type_Oracle
{
    C4V_Any_Oracle = 0,
    C4V_Int_Oracle = 1,
    C4V_Bool_Oracle = 2,
    C4V_C4ID_Oracle = 3,
    C4V_C4Object_Oracle = 4,
    C4V_String_Oracle = 5,
    C4V_Array_Oracle = 6,
    C4V_Map_Oracle = 7,
};

static std::size_t oracleC4Id(std::string_view str)
{
    if (str.size() < 4 || str == "NONE") return 0;

    std::size_t id = 0;
    bool numeric = true;
    for (const auto c : str)
    {
        if (c < '0' || c > '9') { numeric = false; break; }
        id *= 10;
        id += c - '0';
    }
    if (numeric) return id;

    id = 0;
    for (std::size_t i = 4; i > 0; --i)
    {
        id <<= 8;
        id |= static_cast<unsigned char>(str[i - 1]);
    }
    return id;
}

static std::size_t hashInt(int32_t value)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_Int_Oracle);
    hashCombine(hash, std::hash<int32_t>{}(value));
    return hash;
}

static std::size_t hashBool(bool value)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_Bool_Oracle);
    hashCombine(hash, std::hash<bool>{}(value));
    return hash;
}

static std::size_t hashId(std::string_view value)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_C4ID_Oracle);
    hashCombine(hash, std::hash<int32_t>{}(static_cast<int32_t>(oracleC4Id(value))));
    return hash;
}

static std::size_t hashObject(int32_t number)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_C4Object_Oracle);
    hashCombine(hash, std::hash<int32_t>{}(number));
    return hash;
}

static std::size_t hashString(std::string_view value)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_String_Oracle);
    hashCombine(hash, std::hash<std::string_view>{}(value));
    return hash;
}

static std::size_t hashArray(std::initializer_list<std::size_t> values)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_Array_Oracle);
    for (auto value : values) hashCombine(hash, value);
    return hash;
}

static std::size_t hashMap(std::initializer_list<std::pair<std::size_t, std::size_t>> entries)
{
    std::size_t hash = std::hash<C4V_Type_Oracle>{}(C4V_Map_Oracle);
    std::size_t contentHash = 0;
    for (auto [key, value] : entries)
    {
        std::size_t itemHash = key;
        hashCombine(itemHash, value);
        contentHash ^= itemHash; // order mustn't matter
    }
    hashCombine(hash, contentHash);
    return hash;
}

static void printHashCombineCase(const char *name, std::size_t seed, std::size_t nextHash)
{
    std::size_t hash = seed;
    hashCombine(hash, nextHash);
    printf("{\"name\":\"%s\",\"seed\":%zu,\"next\":%zu,\"hash\":%zu}", name, seed, nextHash, hash);
}

static void printHashValueCase(const char *name, std::size_t hash)
{
    printf("{\"name\":\"%s\",\"hash\":%zu}", name, hash);
}

// --- C4ScriptCnvMap: transcribed from src/C4Value.cpp:481-598 ----------------
// The 9x9 type-conversion table and its six converter classes. The real table
// is a private static member of function pointers that cannot be linked here
// without pulling in all of Game/C4Object, so it is transcribed cell-for-cell.
// This is an INDEPENDENT copy from the Rust port's (lc-script/src/value.rs); a
// transcription error on either side surfaces as a divergence below. The
// Game-dependent FnCnvGuess/GuessType branch (C4Value.cpp:299-331) runs only for
// a non-zero C4V_Any value; every oracle input is a concrete type or nil
// (Data==0), so it is never exercised and no engine setup is required.
enum CnvClass { CNV_OK, CNV_ERROR, CNV_GUESS, CNV_INT2ID, CNV_DIRECTOLD, CNV_DEREF };

static const CnvClass C4ScriptCnvMapOracle[9][9] = {
    //   any            int            bool       c4id           object     string     array      map        ref
    { CNV_OK,        CNV_GUESS,     CNV_GUESS, CNV_GUESS,     CNV_GUESS, CNV_GUESS, CNV_GUESS, CNV_GUESS, CNV_ERROR },  // C4V_Any      (:490-501)
    { CNV_OK,        CNV_OK,        CNV_OK,    CNV_INT2ID,    CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR },  // C4V_Int      (:502-513)
    { CNV_OK,        CNV_OK,        CNV_OK,    CNV_DIRECTOLD, CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR },  // C4V_Bool     (:514-525)
    { CNV_OK,        CNV_DIRECTOLD, CNV_OK,    CNV_OK,        CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR },  // C4V_C4ID     (:526-537)
    { CNV_OK,        CNV_DIRECTOLD, CNV_OK,    CNV_ERROR,     CNV_OK,    CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_ERROR },  // C4V_C4Object (:538-549)
    { CNV_OK,        CNV_DIRECTOLD, CNV_OK,    CNV_ERROR,     CNV_ERROR, CNV_OK,    CNV_ERROR, CNV_ERROR, CNV_ERROR },  // C4V_String   (:550-561)
    { CNV_OK,        CNV_ERROR,     CNV_OK,    CNV_ERROR,     CNV_ERROR, CNV_ERROR, CNV_OK,    CNV_ERROR, CNV_ERROR },  // C4V_Array    (:562-573)
    { CNV_OK,        CNV_ERROR,     CNV_OK,    CNV_ERROR,     CNV_ERROR, CNV_ERROR, CNV_ERROR, CNV_OK,    CNV_ERROR },  // C4V_Map      (:574-585)
    { CNV_DEREF,     CNV_DEREF,     CNV_DEREF, CNV_DEREF,     CNV_DEREF, CNV_DEREF, CNV_DEREF, CNV_DEREF, CNV_OK },     // C4V_pC4Value (:586-597)
};

static char cnvCode(CnvClass c)
{
    switch (c)
    {
    case CNV_OK: return 'O';
    case CNV_ERROR: return 'E';
    case CNV_GUESS: return 'G';
    case CNV_INT2ID: return '2';
    case CNV_DIRECTOLD: return 'D';
    case CNV_DEREF: return 'R';
    }
    return '?';
}

// Mirror C4Value::ConvertTo (C4Value.h:248-254): dispatch the cell's converter.
// `intData` is the value's Data.Int (used by Int2Id and the nil/Guess test).
static bool oracleConvertTo(int fromType, int toType, int32_t intData, bool fStrict)
{
    switch (C4ScriptCnvMapOracle[fromType][toType])
    {
    case CNV_OK: return true;                                 // null fn -> true
    case CNV_ERROR: return false;                            // FnCnvError
    case CNV_DIRECTOLD: return !fStrict;                     // FnCnvDirectOld
    case CNV_INT2ID: return intData >= 0 && intData <= 9999; // FnCnvInt2Id
    case CNV_GUESS: return intData == 0;                     // FnCnvGuess: nil (Data==0) is every type
    case CNV_DEREF: return false;                           // FnCnvDeref: no refs among oracle inputs
    }
    return false;
}


// ---------------------------------------------------------------------------
// C4PXSSystem slot allocation (src/C4PXS.cpp). `New` and `Delete` touch only
// the chunk table, so the real bodies run here against this state and nothing
// else: no landscape, no material map, no RNG. Allocation order is what the
// port has to match, because a freed slot is reused at its old index and the
// chunk-major execution order that follows is fixed by it.
namespace pxs_allocation
{
const int32_t MNone = -1;
const size_t PXSChunkSize = 500, PXSMaxChunk = 20;

class C4PXS
{
public:
    C4PXS() : Mat(MNone) {}
    int32_t Mat;
};

class C4PXSSystem
{
public:
    C4PXSSystem()
    {
        for (size_t cnt = 0; cnt < PXSMaxChunk; cnt++)
        {
            Chunk[cnt] = nullptr;
            iChunkPXS[cnt] = 0;
        }
    }

    C4PXS *Chunk[PXSMaxChunk];
    size_t iChunkPXS[PXSMaxChunk];

    C4PXS *New();
    void Delete(C4PXS *pPXS);

    // Where a returned pointer sits, as (chunk, slot). The golden records this
    // rather than the pointer, so the comparison is against slot identity.
    void locate(const C4PXS *pxs, int &chunk, int &slot) const
    {
        for (size_t cnt = 0; cnt < PXSMaxChunk; cnt++)
            if (Chunk[cnt] && pxs >= Chunk[cnt] && pxs < Chunk[cnt] + PXSChunkSize)
            {
                chunk = static_cast<int>(cnt);
                slot = static_cast<int>(pxs - Chunk[cnt]);
                return;
            }
        chunk = -1;
        slot = -1;
    }
};

#include "pxs_new.inc"
#include "pxs_delete.inc"
} // namespace pxs_allocation


// ---------------------------------------------------------------------------
// C4MaterialMap::mrfPoof (src/C4Material.cpp). The mass-move and PXS-position
// arms extract the landscape material, then draw Rnd3 twice: a smoke puff on
// the first zero and a positional sound on the second. Both draws happen
// unconditionally and in that order, which is what makes them synchronised
// state rather than presentation — a port that skips the sound's draw when it
// has no sound to play desynchronises every later draw.
//
// Only the two arms that need no landscape traversal are exercised here.
// `meePXSMove` first runs `mrfInsertCheck`, which walks the landscape through
// `FindMatSlide`, and that wants scaffolding this section deliberately avoids.
namespace poof_reaction
{
enum MaterialInteractionEvent
{
    meePXSPos = 0,
    meePXSMove = 1,
    meeMassMove = 2,
};

struct C4MaterialReaction
{
    bool fUserDefined;
};

// Side effects the arms perform, recorded rather than done.
static int g_extractions = 0;
static int g_smoke = 0;
static int g_sound = 0;

struct LandscapeStub
{
    int32_t ExtractMaterial(int32_t, int32_t)
    {
        g_extractions++;
        return 0;
    }
};

struct GameStub
{
    LandscapeStub Landscape;
} Game;

static void Smoke(int32_t, int32_t, int32_t) { g_smoke++; }
static void StartSoundEffectAt(const char *, int32_t, int32_t) { g_sound++; }

struct C4MaterialMap;

// Named by the lifted body on paths this section does not take: `fUserDefined`
// is always false here, and `meePXSMove` (the only caller of the insert check)
// is excluded because it walks the landscape. Aborting keeps that honest — if
// a future case reaches them, the oracle stops rather than inventing a result.
static bool mrfUserCheck(C4MaterialReaction *, int32_t &, int32_t &, int32_t, int32_t, C4Fixed &,
                         C4Fixed &, int32_t &, int32_t, MaterialInteractionEvent, bool *)
{
    abort();
}

static bool mrfInsertCheck(int32_t &, int32_t &, C4Fixed &, C4Fixed &, int32_t &, int32_t, bool *)
{
    abort();
}

struct C4MaterialMap
{
    bool mrfPoof(C4MaterialReaction *pReaction, int32_t &iX, int32_t &iY, int32_t iLSPosX,
                 int32_t iLSPosY, C4Fixed &fXDir, C4Fixed &fYDir, int32_t &iPxsMat,
                 int32_t iLsMat, MaterialInteractionEvent evEvent, bool *pfPosChanged);
};

#include "mrf_poof.inc"
} // namespace poof_reaction


// ---------------------------------------------------------------------------
// C4MassMoverSet::Create's slot scan (src/C4MassMover.cpp). The search starts
// *after* CreatePtr, wraps at the chunk end, takes the first free slot and
// leaves CreatePtr on it. That is what decides whether the frame's descending
// Execute pass reaches a newly created mover again this pass or only the next
// one, so the sequence of chosen slots is parity state.
//
// `Init` is stubbed to succeed: it does landscape bounds and material work that
// this section deliberately does not scaffold, and the scan is what is being
// pinned. `Execute` is never requested (`fExecute` is false throughout).
namespace mover_allocation
{
const int32_t MNone = -1;
const int32_t C4MassMoverChunk = 10000;

struct C4MassMover
{
    int32_t Mat = MNone;
    int32_t x = 0, y = 0;

    bool Init(int32_t tx, int32_t ty)
    {
        Mat = 1;
        x = tx;
        y = ty;
        return true;
    }

    void Execute() {}
};

struct C4MassMoverSet
{
    int32_t Count = 0;
    int32_t CreatePtr = 0;
    C4MassMover Set[C4MassMoverChunk];

    bool Create(int32_t x, int32_t y, bool fExecute = false);
};

#include "mass_mover_create.inc"
} // namespace mover_allocation


// ---------------------------------------------------------------------------
// Splash (src/C4Effect.cpp), the liquid-entry effect that C4Object's
// UpdateInLiquid and C4Movement's InLiquid check fire on entry. It is lifted
// rather than restated for two reasons: both `Random` pairs are written with an
// explicit r2-before-r1 temporary to force evaluation order, and the extraction
// inside the loop empties the very pixel the liquid test reads, so how many
// draws the call takes depends on the landscape changing underneath it.
//
// The scaffold is an 8x40 material grid. `BubbleOut`, `PXS::Create` and
// `ExtractMaterial` record into fixed buffers rather than simulate;
// `StartSoundEffect` is presentation and only records which cue was chosen.
namespace splash_effect
{
struct C4Object;

const int32_t MNone = -1;
const int32_t GridWdt = 8;
const int32_t GridHgt = 40;
const int32_t MaxEvents = 64;

struct MatEntry
{
    int32_t Density;
    bool Instable;
};

// 0: water (liquid and instable, the only combination Splash acts on),
// 1: a liquid that is NOT instable, 2: granite.
static MatEntry g_map[3] = {{25, true}, {25, false}, {50, false}};

static int32_t g_grid[GridHgt][GridWdt];

struct Bubble
{
    int32_t x, y;
};

struct Cast
{
    int32_t mat, x, y, xdir, ydir;
};

static Bubble g_bubbles[MaxEvents];
static Cast g_casts[MaxEvents];
static int32_t g_bubble_count = 0;
static int32_t g_cast_count = 0;
static int32_t g_extractions = 0;
static const char *g_sound = "";

struct LandscapeStub
{
    int32_t ExtractMaterial(int32_t x, int32_t y);
};

struct PXSStub
{
    void Create(int32_t mat, C4Fixed x, C4Fixed y, C4Fixed xdir, C4Fixed ydir);
};

struct GameStub
{
    struct
    {
        MatEntry *Map = g_map;
    } Material;

    LandscapeStub Landscape;
    PXSStub PXS;
};

static GameStub Game;

static bool MatValid(int32_t mat) { return mat >= 0 && mat < 3; }
static bool DensityLiquid(int32_t dens) { return dens >= 25 && dens < 50; }
static bool DensitySemiSolid(int32_t dens) { return dens >= 25; }

static int32_t GBackMat(int32_t x, int32_t y)
{
    if (x < 0 || y < 0 || x >= GridWdt || y >= GridHgt) return MNone;
    return g_grid[y][x];
}

static int32_t GBackDensity(int32_t x, int32_t y)
{
    const int32_t mat = GBackMat(x, y);
    return MatValid(mat) ? g_map[mat].Density : 0;
}

static bool GBackLiquid(int32_t x, int32_t y) { return DensityLiquid(GBackDensity(x, y)); }
static bool GBackSemiSolid(int32_t x, int32_t y) { return DensitySemiSolid(GBackDensity(x, y)); }

int32_t LandscapeStub::ExtractMaterial(int32_t x, int32_t y)
{
    ++g_extractions;
    if (!GBackLiquid(x, y)) return MNone;
    const int32_t mat = g_grid[y][x];
    g_grid[y][x] = MNone;
    return mat;
}

void PXSStub::Create(int32_t mat, C4Fixed x, C4Fixed y, C4Fixed xdir, C4Fixed ydir)
{
    // The real C4PXSSystem::Create rejects an invalid material before doing
    // anything (C4PXS.cpp:210), and Splash hands it ExtractMaterial's result
    // unconditionally. No case here reaches that, since the extraction is
    // guarded by the same liquid test, but the stub must not record a cast the
    // engine would have dropped.
    if (!MatValid(mat)) return;
    if (g_cast_count < MaxEvents)
        g_casts[g_cast_count] = {mat, fixtoi(x), fixtoi(y), fixtoi(xdir, 100), fixtoi(ydir, 100)};
    ++g_cast_count;
}

static void BubbleOut(int32_t tx, int32_t ty)
{
    if (g_bubble_count < MaxEvents) g_bubbles[g_bubble_count] = {tx, ty};
    ++g_bubble_count;
}

static void StartSoundEffect(const char *name, bool, int32_t, C4Object *) { g_sound = name; }

#include "splash.inc"

// Water everywhere at or below `water_top`, granite below `floor_top`, with an
// optional granite plug at (plug_x, plug_y) and an optional non-instable liquid
// column. Everything else is sky.
static void reset_grid(int32_t water_top, int32_t floor_top, int32_t liquid_mat)
{
    for (int32_t y = 0; y < GridHgt; ++y)
        for (int32_t x = 0; x < GridWdt; ++x)
            g_grid[y][x] = y >= floor_top ? 2 : (y >= water_top ? liquid_mat : MNone);
    g_bubble_count = 0;
    g_cast_count = 0;
    g_extractions = 0;
    g_sound = "";
}

// C4Object::UpdateInLiquid and IsInLiquidCheck live in the same namespace so
// that the splash they fire on entry is the real `Splash` above, over the same
// grid, rather than a second stand-in. Only the fields the two bodies read are
// scaffolded; `Def->Float` and `Con` are what move the probe off the object's
// own y.
const int32_t FullCon = 100000;
const uint32_t OCF_HitSpeed2 = 1 << 12;

struct DefStub
{
    int32_t Float = 0;
};

struct ShapeStub
{
    int32_t Wdt = 0;
    int32_t Hgt = 0;
};

struct C4Object
{
    int32_t x = 0;
    int32_t y = 0;
    int32_t Con = FullCon;
    int32_t Mass = 0;
    int32_t InLiquid = 0;
    uint32_t OCF = 0;
    ShapeStub Shape;
    DefStub *Def = nullptr;

    bool IsInLiquidCheck();
    void UpdateInLiquid();
};

#include "is_in_liquid_check.inc"
#include "update_in_liquid.inc"

} // namespace splash_effect


// ---------------------------------------------------------------------------
// C4Weather::Execute (src/C4Weather.cpp). The disaster block is what makes this
// determinism-critical: four gates in a fixed order, each drawing its
// `Random(100)` level test EVEN WHEN the level is zero, so the number of draws
// a tick takes is decided by which outer gates hit rather than by which
// disasters happen. Three of the four then use the same forced r2-before-r1
// evaluation order seen elsewhere in the engine.
//
// Scaffolded rather than run: `SetSeasonGamma` (with `NoGamma`, which
// C4Weather::Default sets, the production body returns immediately at
// C4Weather.cpp:261, so an empty stub is exact for this configuration),
// `SoundLevel` (presentation), and the three `Launch*` helpers plus
// `CreateObject` (their production bodies are a CreateObject and an Activate
// call with no draws of their own, so recording their arguments is what they
// would have passed on). Nothing here consumes RNG that the production path
// would not.
namespace weather_execute
{
// The lifted body declares `C4Object *meto;`; nothing is ever read back through
// it, so an incomplete type is all the scaffold owes it.
struct C4Object;

int32_t Tick10 = 1;
int32_t Tick35 = 1;
int32_t Tick1000 = 1;

const int32_t GBackWdt = 200;
const int32_t GBackHgt = 100;
const int32_t NO_OWNER = -1;
const int32_t MNone = -1;
const int32_t LavaMaterial = 3;

struct C4SVal
{
    int32_t Std{};
    int32_t Rnd{};
    int32_t Min{};
    int32_t Max{};

    int32_t Evaluate();
};

#include "c4sval_evaluate.inc"

// Recorded effects, in the order Execute produces them.
struct Event
{
    const char *kind;
    int32_t a, b, c, d;
};

const int32_t MaxEvents = 32;
static Event g_events[MaxEvents];
static int32_t g_event_count = 0;

static void record(const char *kind, int32_t a, int32_t b, int32_t c, int32_t d)
{
    if (g_event_count < MaxEvents) g_events[g_event_count] = {kind, a, b, c, d};
    ++g_event_count;
}

struct GameStub
{
    struct
    {
        struct
        {
            C4SVal StartSeason;
            C4SVal Wind;
        } Weather;
    } C4S;

    struct
    {
        int32_t TopOpen{};
    } Landscape;

    struct
    {
        int32_t Get(const char *name) { return name[0] == 'L' ? LavaMaterial : MNone; }
    } Material;

    // The meteor arm's CreateObject. Only the arguments are kept: the
    // production call returns an object this block never reads again.
    C4Object *CreateObject(unsigned long, void *, int32_t, int32_t x, int32_t y, int32_t,
                           C4Fixed xdir, C4Fixed ydir, C4Fixed rdir)
    {
        record("meteorite", x, y, xdir.val, ydir.val);
        record("meteorite_rdir", rdir.val, 0, 0, 0);
        return nullptr;
    }
};

static GameStub Game;

static void SoundLevel(const char *, void *, int32_t) {}

struct C4Weather
{
    int32_t Season{};
    int32_t YearSpeed{};
    int32_t SeasonDelay{};
    int32_t Wind{};
    int32_t TargetWind{};
    int32_t Temperature{};
    int32_t Climate{};
    int32_t TemperatureRange{30};
    int32_t MeteoriteLevel{};
    int32_t VolcanoLevel{};
    int32_t EarthquakeLevel{};
    int32_t LightningLevel{};
    bool NoGamma{true};

    void Execute();
    void SetSeasonGamma() {}

    bool LaunchLightning(
        int32_t x, int32_t y, int32_t xdir, int32_t xrange, int32_t ydir, int32_t yrange, bool)
    {
        record("lightning", x, y, xdir, ydir);
        return true;
    }

    bool LaunchEarthquake(int32_t x, int32_t y)
    {
        record("earthquake", x, y, 0, 0);
        return true;
    }

    bool LaunchVolcano(int32_t mat, int32_t x, int32_t y, int32_t size)
    {
        record("volcano", x, y, size, mat);
        return true;
    }
};

const unsigned long C4ID_Meteor = 0;

#include "weather_execute.inc"
} // namespace weather_execute


// ---------------------------------------------------------------------------
// C4Shape::ContactCheck (src/C4Shape.cpp), the per-pixel probe every step of
// C4Object::DoMovement runs. Its density reads go through GetPix's border rules
// (C4Landscape.h:163-180), where a CLOSED border answers MCVehic — solid —
// instead of sky, so an object walking off the map edge stops there rather than
// falling out of the world.
namespace shape_contact
{
const int32_t GridWdt = 24;
const int32_t GridHgt = 16;
const int32_t MNone = -1;
const int32_t MVehicle = 3;
const int32_t SolidDensity = 50;   // C4M_Solid, C4Material.h:201
const int32_t VehicleDensity = 100; // C4M_Vehicle, C4Material.h:200
const int32_t AttachRange = 5;      // C4Physics.h:24
const int32_t GBackWdt = GridWdt;   // C4Wrappers.h:86

// 0 sky, 1 earth, 2 water, 3 vehicle (what a closed border answers).
const int32_t g_density[4] = {0, 50, 30, VehicleDensity};
static int32_t g_grid[GridHgt][GridWdt];

// C4Landscape's border configuration: LeftOpen/RightOpen are HEIGHTS (a pixel
// is open while y is above them), TopOpen/BottomOpen are flags.
static int32_t g_left_open = 0;
static int32_t g_right_open = 0;
static bool g_top_open = true;
static bool g_bottom_open = false;

// The GetPix border cascade (C4Landscape.h:163-180). The grid stores texmap
// bytes, and this fixture maps byte to material one to one, so PixCol2Mat is
// the identity outside sky.
static uint8_t GBackPix(int32_t x, int32_t y)
{
    if (x < 0) return y < g_left_open ? 0 : MVehicle;
    if (x >= GridWdt) return y < g_right_open ? 0 : MVehicle;
    if (y < 0) return g_top_open ? 0 : MVehicle;
    if (y >= GridHgt) return g_bottom_open ? 0 : MVehicle;
    return static_cast<uint8_t>(g_grid[y][x]);
}

static int32_t PixCol2Mat(uint8_t pix) { return pix ? pix : MNone; }

static int32_t MatDensity(int32_t mat) { return mat < 0 ? 0 : g_density[mat]; }

static int32_t GBackMat(int32_t x, int32_t y) { return PixCol2Mat(GBackPix(x, y)); }

static int32_t GBackDensity(int32_t x, int32_t y) { return MatDensity(GBackMat(x, y)); }

const int32_t MaxVertex = 8;

struct C4Shape
{
    int32_t VtxNum{};
    int32_t VtxX[MaxVertex]{};
    int32_t VtxY[MaxVertex]{};
    int32_t VtxCNAT[MaxVertex]{};
    int32_t VtxContactCNAT[MaxVertex]{};
    int32_t VtxContactMat[MaxVertex]{};
    int32_t ContactDensity{SolidDensity};
    int32_t ContactCNAT{};
    int32_t ContactCount{};

    // C4Shape::Attach's outputs (C4Shape.cpp:176, 217-219, 253-255).
    int32_t AttachMat{MNone};
    int32_t iAttachX{};
    int32_t iAttachY{};
    int32_t iAttachVtx{};

    bool ContactCheck(int32_t cx, int32_t cy);
    bool Attach(int32_t &cx, int32_t &cy, uint8_t cnat_pos);
};

#include "shape_contact_check.inc"
#include "shape_attach.inc"

// C4Object::TargetBounds (C4Movement.cpp:128-164), the clamp SideBounds and
// VerticalBounds run the movement target through. It decides which velocity
// component is zeroed and fires a Contact call per bound crossed — and when the
// two limits cross each other, BOTH fire, in low-then-high order.
struct C4Object
{
    C4Fixed xdir{Fix0};
    C4Fixed ydir{Fix0};
    int32_t ContactCalls[4]{};
    int32_t ContactCallCount{};

    bool Contact(int32_t cnat)
    {
        if (ContactCallCount < 4) ContactCalls[ContactCallCount] = cnat;
        ++ContactCallCount;
        return false;
    }

    void TargetBounds(
        int32_t &ctco, int32_t limit_low, int32_t limit_hi, int32_t cnat_low, int32_t cnat_hi);
};

#include "target_bounds.inc"
} // namespace shape_contact


// ---------------------------------------------------------------------------
// The container lifecycle: C4Object::Enter, Exit and Collect (src/C4Object.cpp).
// What is pinned here is the SHAPE of three ordered state machines — which
// script call runs before which mutation, which rollback undoes a failed
// insert, and which `Status` re-check abandons the rest after a callback
// removed one of the two objects.
//
// `C4ObjectList::Add`'s insert ORDER is deliberately not modelled: it sorts by
// category through a linked list with its own invariants, and lifting it would
// be a section of its own. `Contents` here is an append-only list whose `Add`
// can be told to fail, which is what exercises Enter's rollback. Contents
// ordering keeps its existing Rust-side coverage.
//
// Script calls are recorded rather than executed, and each can be configured to
// return a value and to perform one side effect — clearing either object's
// Status, or re-entering the object elsewhere — so the re-checks after
// Collection2, Entrance, Ejection and Departure are reachable.
namespace container_lifecycle
{
struct C4Object;

const int32_t ActIdle = -1;
const int32_t C4D_Living = 1 << 3;             // C4Def.h:47
const int32_t C4ID_Flag = 1;                   // stands in for C4Id("FLAG")
const int32_t C4RULE_FlagRemoveable = 1 << 0;  // C4Rules bit under test
// OCF_HitSpeed1..3 and BASEFUNC_AutoSellContents come from the real
// C4Constants.h / C4Scenario.h the oracle already includes.

// The PSF_ names the lifted bodies call, copied verbatim from C4Script.h
// (:48-50, 56-60, 82, 96). The leading `~` marks the callback optional and is
// part of the name the engine looks up — note that the collection veto is
// spelled `RejectCollect`, NOT `RejectCollection`.
#define PSF_Hit "~Hit"
#define PSF_Hit2 "~Hit2"
#define PSF_Hit3 "~Hit3"
#define PSF_Collection "~Collection"
#define PSF_Collection2 "~Collection2"
#define PSF_Ejection "~Ejection"
#define PSF_Entrance "~Entrance"
#define PSF_Departure "~Departure"
#define PSF_RejectCollection "~RejectCollect"
#define PSF_RejectEntrance "~RejectEntrance"

struct C4Value
{
};

static C4Value C4VObj(C4Object *) { return {}; }
static C4Value C4VID(int32_t) { return {}; }

struct ParSet
{
    ParSet() {}
    ParSet(std::initializer_list<C4Value>) {}
};

// What a configured callback does besides returning its value.
enum class Effect
{
    None,
    ClearSelfStatus,
    ClearOtherStatus,
    ClearContainer,
    ReEnter,
    // A container callback (Collection2) that removes the object that just
    // entered — the case Enter's post-callback re-check exists for.
    ClearEnteringContainer,
    ClearEnteringStatus,
    // A callback that Exits the object it was told about, running the lifted
    // Exit body — which is what a script doing the same would do, callbacks and
    // all.
    ExitEntering,
};

// The container an Effect::ReEnter callback drops the object into, standing in
// for a script that called Enter from inside Departure.
static C4Object *g_reenter_target = nullptr;

// The object being entered/collected, so a callback made ON THE CONTAINER can
// still act on it.
static C4Object *g_entering_object = nullptr;

struct CallConfig
{
    const char *tag;
    const char *fn;
    int32_t result;
    Effect effect;
};

const int32_t MaxCalls = 32;
const int32_t MaxConfigs = 8;

static CallConfig g_configs[MaxConfigs];
static int32_t g_config_count = 0;
static const char *g_calls[MaxCalls];
static int32_t g_call_count = 0;

struct DefStub
{
    int32_t id{};
    // The C4Def state C4Object::ChangeDef moves across.
    int32_t Count{};
    bool Rotateable{};
    int32_t BlitMode{};
    int32_t SolidMask{};
    int32_t Graphics{};

    struct
    {
        int32_t LocalNamed{};
    } Script;

    struct ActMapEntry
    {
        const char *Name{""};
    } ActMap[4];
};

// The lifted ChangeDef body names C4Def and C4ID.
using C4Def = DefStub;
using C4ID = int32_t;

// The two definitions a ChangeDef case moves between, resolved by id.
static DefStub *g_definitions[2] = {nullptr, nullptr};

static DefStub *C4Id2Def(int32_t id)
{
    for (DefStub *definition : g_definitions)
        if (definition && definition->id == id) return definition;
    return nullptr;
}

const int32_t C4GFXBLIT_CUSTOM = 1 << 4; // C4Def.h

struct PlayerColorStub
{
    int32_t ColorDw{};
};

struct PlayerColorListStub
{
    PlayerColorStub *Held{};

    PlayerColorStub *Get(int32_t owner) { return owner >= 0 ? Held : nullptr; }
};



struct GameStub
{
    int32_t Rules{};
    struct
    {
        struct
        {
            struct
            {
                int32_t BaseFunctionality{};
            } Realism;
        } Game;
    } C4S;
};

static GameStub Game;

static bool ValidPlr(int32_t player) { return player >= 0; }
// SEqual now comes from the lifted C4Strings helpers, which additionally treat
// a null operand as unequal; every call here passes a literal.

struct ContentsList
{
    C4Object *Items[8]{};
    int32_t Count{};
    bool RefuseAdd{false};

    bool Add(C4Object *object, int32_t)
    {
        if (RefuseAdd || Count >= 8) return false;
        Items[Count++] = object;
        return true;
    }

    void Remove(C4Object *object)
    {
        int32_t write = 0;
        for (int32_t read = 0; read < Count; ++read)
            if (Items[read] != object) Items[write++] = Items[read];
        Count = write;
    }
};

struct C4ObjectList
{
    enum SortType
    {
        stNone = 0,
        stMain = 1,
        stContents = 2,
        stReverse = 3,
    };
};

// ChangeDef walks the master list to tell every object's effects that a
// definition changed, so the link type has to be this namespace's.
struct C4ObjectLink
{
    C4Object *Obj{};
    C4ObjectLink *Next{};
};

struct C4Object
{
    const char *Tag{""};
    int32_t Status{1};
    C4Object *Contained{};
    ContentsList Contents;
    DefStub *Def{};
    int32_t Alive{};
    int32_t Category{};
    int32_t Controller{-1};
    int32_t Base{-1};
    uint32_t OCF{};
    int32_t Mobile{};
    int32_t InLiquid{};
    int32_t x{}, y{}, r{};
    C4Fixed fix_x{Fix0}, fix_y{Fix0}, fix_r{Fix0};
    C4Fixed xdir{Fix0}, ydir{Fix0}, rdir{Fix0};

    struct ActionState
    {
        int32_t Act{ActIdle};
    } Action;

    int32_t Call(const char *fn, ParSet = {});

    // Everything below is bookkeeping the lifted bodies invoke but this section
    // does not pin; each records that it ran so a reordering is still visible.
    void CloseMenu(bool) { record("CloseMenu"); }
    void SetOCF() { record("SetOCF"); }
    void UpdateFace(bool) { record("UpdateFace"); }
    void UpdateMass() { record("UpdateMass"); }
    void UpdateSolidMask(bool) { record("UpdateSolidMask"); }
    void CopyMotion(C4Object *) { record("CopyMotion"); }
    void BoundsCheck(int32_t &, int32_t &) { record("BoundsCheck"); }
    void AutoSellContents() { record("AutoSellContents"); }

    // The rest of what C4Object::ChangeDef touches. `Unsorted` and the
    // graphics/blit/colour fields are plain state the change carries across;
    // the update chain records that it ran, in order.
    int32_t id{};
    int32_t Owner{-1};
    int32_t Color{};
    int32_t BlitMode{};
    int32_t SolidMask{};
    bool Unsorted{};
    int32_t *pGraphics{};

    // Every object's effects are told the definition changed; the scaffold
    // records that walk rather than modelling effect internals.
    struct EffectListStub
    {
        void OnObjectChangedDef(C4Object *) { record("EffectsOnChangedDef"); }
    } *pEffects{};

    struct SolidMaskDataStub
    {
        void Remove(bool, bool) { record("SolidMaskRemove"); }
    } *pSolidMaskData{};

    struct LocalNamedStub
    {
        int32_t *List{};

        void SetNameList(int32_t *list) { List = list; }
    } LocalNamed;

    void SetAction(int32_t) { record("SetActionIdle"); }
    void SetDir(int32_t) { record("SetDir"); }
    void UpdateGraphics(bool) { record("UpdateGraphics"); }

    bool ChangeDef(C4ID idNew);

    static void record(const char *what)
    {
        if (g_call_count < MaxCalls) g_calls[g_call_count] = what;
        ++g_call_count;
    }

    bool Enter(
        C4Object *pTarget, bool fCalls = true, bool fCopyMotion = true,
        bool *pfRejectCollect = nullptr);
    bool Exit(
        int32_t iX = 0, int32_t iY = 0, int32_t iR = 0, C4Fixed iXDir = Fix0,
        C4Fixed iYDir = Fix0, C4Fixed iRDir = Fix0, bool fCalls = true);
    bool Collect(C4Object *pObj);
};

static void ObjectComCancelAttach(C4Object *) { C4Object::record("CancelAttach"); }

inline int32_t C4Object::Call(const char *fn, ParSet)
{
    record(fn);
    for (int32_t i = 0; i < g_config_count; ++i)
    {
        if (!SEqual(g_configs[i].tag, Tag) || !SEqual(g_configs[i].fn, fn)) continue;
        switch (g_configs[i].effect)
        {
        case Effect::ClearSelfStatus: Status = 0; break;
        case Effect::ClearOtherStatus:
            if (Contained) Contained->Status = 0;
            break;
        case Effect::ClearContainer: Contained = nullptr; break;
        case Effect::ReEnter:
            // Run the real Enter, as a script calling it from inside Departure
            // would: the whole point is that Exit then reports failure.
            if (g_entering_object) g_entering_object->Enter(g_reenter_target);
            break;
        case Effect::ClearEnteringContainer:
            if (g_entering_object) g_entering_object->Contained = nullptr;
            break;
        case Effect::ClearEnteringStatus:
            if (g_entering_object) g_entering_object->Status = 0;
            break;
        case Effect::ExitEntering:
            if (g_entering_object) g_entering_object->Exit();
            break;
        case Effect::None: break;
        }
        return g_configs[i].result;
    }
    return 0;
}

#include "object_exit.inc"
#include "object_enter.inc"
#include "object_collect.inc"

// C4Object::ChangeDef, compiled beside the real Enter/Exit so its container
// round-trip runs the production bodies with fCalls=false — which is exactly
// the fact worth pinning: a definition change inside a container fires neither
// Ejection/Departure on the way out nor Collection2/Entrance on the way back.
static PlayerColorListStub g_player_colors;

struct ChangeDefGameStub
{
    PlayerColorListStub &Players = g_player_colors;

    struct
    {
        C4ObjectLink *First{};
    } Objects;

    C4Object::EffectListStub *pGlobalEffects{};
};

static ChangeDefGameStub ChangeDefGame;

#define Game ChangeDefGame
#include "object_change_def.inc"
#undef Game
} // namespace container_lifecycle


// ---------------------------------------------------------------------------
// C4Effect::Check (src/C4Effect.cpp), the negotiation every AddEffect runs
// before a new effect exists. The branch it takes decides whether the effect is
// created at all, absorbed into an existing one, or denied outright — and the
// AnnulCalls form brackets its FxAdd in temp-remove/temp-readd of the effects
// above the absorber.
//
// Each scaffolded effect answers the checker with a configured value, and every
// call the body makes is recorded, so the SEQUENCE is what the section pins.
namespace effect_check
{
struct C4Object;
struct C4Effect;

struct FnTimer;

// C4Effects.h:34-43.
const int32_t C4Fx_OK = 0;
const int32_t C4Fx_Effect_Deny = -1;
const int32_t C4Fx_Effect_Annul = -2;
const int32_t C4Fx_Effect_AnnulCalls = -3;
const int32_t C4Fx_Start_Deny = -1;

#define PSFS_FxAdd "Add"

// The lifted Execute `delete`s the effects it unlinks, so the scaffold's
// effects are heap-allocated and their destruction is counted.
static int32_t g_deleted = 0;

const int32_t MaxTrace = 32;
static const char *g_trace[MaxTrace];
static int32_t g_trace_count = 0;

static void trace(const char *what)
{
    if (g_trace_count < MaxTrace) g_trace[g_trace_count] = what;
    ++g_trace_count;
}

struct C4Value
{
    int32_t value{};

    int32_t getInt() const { return value; }
};

static C4Value C4VString(const char *) { return {}; }
static C4Value C4VObj(C4Object *) { return {}; }
static C4Value C4VInt(int32_t v) { return {v}; }

struct ParSet
{
    ParSet() {}
    ParSet(std::initializer_list<C4Value>) {}
};

// The checker's answer for one effect, plus what its FxAdd returns when it is
// the one that absorbs the newcomer.
struct EffectConfig
{
    const char *name;
    int32_t priority;
    bool dead;
    bool has_function;
    int32_t effect_result;
    int32_t add_result;
};

struct C4Effect;

struct FnEffect
{
    C4Effect *owner{};

    C4Value Exec(C4Object *, ParSet, bool, bool);
};

struct FnTimer
{
    C4Effect *owner{};

    C4Value Exec(C4Object *, ParSet, bool, bool);
};

struct C4Effect
{
    const char *Name{""};
    int32_t iPriority{};
    int32_t iNumber{};
    int32_t EffectResult{C4Fx_OK};
    int32_t AddResult{C4Fx_OK};
    bool Killed{};
    C4Effect *pNext{};
    C4Object *pCommandTarget{};
    FnEffect *pFnEffect{};
    FnTimer *pFnTimer{};
    int32_t iTime{};
    int32_t iIntervall{};
    int32_t TimerResult{C4Fx_OK};

    // C4Effects.h:110 — a dead effect is one whose priority was zeroed, not a
    // separate flag, which is also how the port marks it.
    ~C4Effect() { ++g_deleted; }

    // C4Effects.h:110 — a dead effect is one whose priority was zeroed, not a
    // separate flag, which is also how the port marks it.
    bool IsDead() const { return !iPriority; }

    C4Value DoCall(C4Object *, const char *fn, C4Value &, C4Value &, const C4Value &,
                   const C4Value &, const C4Value &, const C4Value &)
    {
        trace(fn);
        return {AddResult};
    }

    void Kill(C4Object *)
    {
        trace("Kill");
        Killed = true;
        iPriority = 0;
    }

    void TempRemoveUpperEffects(C4Object *, bool, C4Effect **ppLastRemovedEffect)
    {
        trace("TempRemoveUpper");
        if (ppLastRemovedEffect) *ppLastRemovedEffect = pNext;
    }

    void TempReaddUpperEffects(C4Object *, C4Effect *) { trace("TempReaddUpper"); }

    int32_t Check(
        C4Object *pForObj, const char *szCheckEffect, int32_t iPrio, int32_t iTimer,
        const C4Value &rVal1, const C4Value &rVal2, const C4Value &rVal3, const C4Value &rVal4,
        bool passErrors);
    void Execute(C4Object *pObj);
};

// C4Effects.h: the timer's "finish me" answer.
const int32_t C4Fx_Execute_Kill = -1;



C4Value FnEffect::Exec(C4Object *, ParSet, bool, bool)
{
    trace(owner->Name);
    return {owner->EffectResult};
}

// C4Effect::Execute's per-frame pass. It walks the list unlinking dead effects
// as it goes, advances each survivor's clock, and fires the timer only on an
// exact interval boundary — or kills the effect outright when it has no timer
// function at all. Only the two members it reads are scaffolded on C4Object.
struct C4Object
{
    int32_t Status{1};
    C4Effect *pEffects{};
};

struct GameStub
{
    C4Effect *pGlobalEffects{};
};

static GameStub Game;

C4Value FnTimer::Exec(C4Object *, ParSet, bool, bool)
{
    trace(owner->Name);
    return {owner->TimerResult};
}

#include "effect_execute.inc"

#include "effect_check.inc"
} // namespace effect_check


// ---------------------------------------------------------------------------
// C4Object::AssignRemoval (src/C4Object.cpp), the object teardown. Its shape is
// the parity fact: the container's ContentsDestruction runs before the object's
// own Destruction, effects are cleared next, and EVERY one of those steps is
// followed by a `Status` re-check because the callback may already have deleted
// the object. The contents are then torn down BEFORE the object leaves its own
// container — reversing those two would give a dying object's cargo a different
// container to exit into.
//
// `fExitContents` chooses whether the cargo is Exited or removed recursively,
// which is the difference between a destroyed lorry spilling its load and
// taking it with it.
namespace object_removal
{
struct C4Object;

#define PSF_ContentsDestruction "~ContentsDestruction"
#define PSF_Destruction "~Destruction"

const int32_t C4OS_DELETED = 0;
const int32_t C4OS_NORMAL = 1;
const int32_t C4OS_INACTIVE = 2;
const int32_t ActIdle = -1;
const int32_t C4FxCall_RemoveClear = 5; // C4Effects.h

const int32_t MaxTrace = 48;
static const char *g_trace[MaxTrace];
static int32_t g_trace_count = 0;

static void trace(const char *what)
{
    if (g_trace_count < MaxTrace) g_trace[g_trace_count] = what;
    ++g_trace_count;
}

struct C4Value
{
    int32_t value{};
};

static C4Value C4VObj(C4Object *) { return {}; }
static C4Value C4VInt(int32_t value) { return {value}; }

// Only the first parameter is kept, which is all these callbacks carry that
// this section compares — PSF_Death's death-causing player.
struct ParSet
{
    int32_t First{};
    bool Any{};

    ParSet() {}

    ParSet(std::initializer_list<C4Value> values)
    {
        if (values.size())
        {
            First = values.begin()->value;
            Any = true;
        }
    }
};

// What a configured callback does besides being recorded.
enum class Effect
{
    None,
    ClearSelfStatus,
    // A callback made ON THE CONTAINER that removes the object being torn
    // down — the case the re-check after ContentsDestruction exists for.
    ClearRemovingStatus,
    // A callback that removes the object being torn down by calling the real
    // teardown on it, which is what a script doing the same would do —
    // callbacks and all — rather than zeroing a flag.
    RemoveRemoving,
};

struct CallConfig
{
    const char *tag;
    const char *fn;
    Effect effect;
    // A nested teardown would re-enter the same callback forever; each
    // configured effect fires once.
    bool fired;
};

// The object currently being removed, so a container's callback can reach it.
static C4Object *g_removing = nullptr;

const int32_t MaxConfigs = 4;
static CallConfig g_configs[MaxConfigs];
static int32_t g_config_count = 0;

struct DefStub
{
    int32_t Count{1};
    bool Line{};
};

struct C4ObjectLink
{
    C4Object *Obj{};
    C4ObjectLink *Next{};
};

struct ContentsList
{
    C4ObjectLink *First{};

    void Add(C4Object *object);
    void Remove(C4Object *object);
    int32_t Count() const;
    C4Object *GetObject() const { return First ? First->Obj : nullptr; }
};

// The two particle chunks the teardown clears. Only the emptiness test and the
// clear are modelled; particles are presentation.
struct ParticleList
{
    bool Present{};

    explicit operator bool() const { return Present; }

    void Clear()
    {
        trace("ParticlesClear");
        Present = false;
    }
};

struct EffectsStub
{
    // When set, the clear puts the object back on its feet — the resurrection
    // C4Object::AssignDeath aborts an unforced kill for.
    bool Resurrects{};

    void ClearAll(C4Object *object, int32_t);
};

// C4Effects.h: the reason code a death-driven effect clear passes on.
const int32_t C4FxCall_RemoveDeath = 6;
const int32_t C4D_Living = 1 << 3; // C4Def.h:47

// Only the two members AssignDeath reads: the pointer cleanup and the
// fog-of-war view list that decides whether the view range is reset.
struct C4Player
{
    struct ViewList
    {
        bool Contains{};

        bool IsContained(C4Object *) const { return Contains; }
    } FoWViewObjs;

    void ClearPointers(C4Object *, bool) { trace("PlayerClearPointers"); }
};

struct PlayerListStub
{
    C4Player *Held{};

    C4Player *Get(int32_t owner) { return owner >= 0 ? Held : nullptr; }
};

struct InactiveList
{
    bool Held{};

    void Remove(C4Object *) { trace("InactiveRemove"); }
};

struct ObjectsStub
{
    InactiveList InactiveObjects;

    void Add(C4Object *) { trace("MainListAdd"); }
};

struct GameStub
{
    ObjectsStub Objects;
    PlayerListStub Players;

    void ClearPointers(C4Object *) { trace("ClearPointers"); }
};

static GameStub Game;

struct InfoStub
{
    bool HasDied{};
    int32_t DeathCount{};

    void Retire() { trace("InfoRetire"); }
};



struct C4Object
{
    const char *Tag{""};
    int32_t Status{C4OS_NORMAL};
    C4Object *Contained{};
    ContentsList Contents;
    EffectsStub *pEffects{};
    DefStub *Def{};
    InfoStub *Info{};

    // The reference chain the teardown zeroes, and the solid mask it drops.
    // Both are pointer bookkeeping outside this section; the stubs record that
    // they ran, and the reference pops itself so the production
    // `while (FirstRef)` loop terminates.
    struct RefStub
    {
        RefStub *NextRef{};

        void Set0();
    } *FirstRef{};

    struct SolidMaskStub
    {
        void Remove(bool, bool) { trace("SolidMaskRemove"); }
    } *pSolidMaskData{};

    ParticleList FrontParticles;
    ParticleList BackParticles;
    int32_t RemovalDelay{};
    int32_t x{}, y{};

    struct
    {
        int32_t Wdt{};
    } SolidMask;

    // C4Object::AssignDeath's state.
    int32_t Alive{1};
    int32_t Select{};
    int32_t Owner{-1};
    int32_t Category{};
    int32_t LastEnergyLossCausePlayer{-1};
    int32_t DeathPlayerSeen{-1};

    int32_t Call(const char *fn, ParSet = {});

    void AssignDeath(bool fForced = false);
    bool SetActionByName(const char *name)
    {
        trace(name);
        return true;
    }
    void SetPlrViewRange(int32_t) { trace("SetPlrViewRange"); }

    void UpdateMass() { trace("UpdateMass"); }
    void SetOCF() { trace("SetOCF"); }
    void SetAction(int32_t) { trace("SetActionIdle"); }
    void ClearCommands() { trace("ClearCommands"); }
    bool Exit(int32_t, int32_t)
    {
        trace("ContentExit");
        if (Contained) Contained->Contents.Remove(this);
        Contained = nullptr;
        return true;
    }

    void AssignRemoval(bool fExitContents = false);
};

void ContentsList::Add(C4Object *object)
{
    C4ObjectLink **tail = &First;
    while (*tail) tail = &(*tail)->Next;
    *tail = new C4ObjectLink{object, nullptr};
}

void ContentsList::Remove(C4Object *object)
{
    C4ObjectLink **link = &First;
    while (*link)
    {
        if ((*link)->Obj == object)
        {
            C4ObjectLink *dead = *link;
            *link = dead->Next;
            delete dead;
            return;
        }
        link = &(*link)->Next;
    }
}

int32_t ContentsList::Count() const
{
    int32_t count = 0;
    for (C4ObjectLink *link = First; link; link = link->Next) ++count;
    return count;
}

// The owner whose reference chain a Set0 pops, so the production
// `while (FirstRef)` loop ends.
static C4Object *g_ref_owner = nullptr;

void C4Object::RefStub::Set0()
{
    trace("RefSet0");
    if (g_ref_owner) g_ref_owner->FirstRef = NextRef;
}

int32_t C4Object::Call(const char *fn, ParSet pars)
{
    // C4Object::Call (C4Object.cpp:2224-2228) drops the call outright when the
    // callee's Status is zero, so a container that is itself already torn down
    // receives nothing — the teardown reaches the call site either way, which
    // is why the guard belongs here rather than at the caller.
    if (!Status || !Def) return 0;
    if (pars.Any) DeathPlayerSeen = pars.First;
    trace(fn);
    for (int32_t i = 0; i < g_config_count; ++i)
    {
        if (std::strcmp(g_configs[i].tag, Tag) != 0 || std::strcmp(g_configs[i].fn, fn) != 0)
            continue;
        if (g_configs[i].fired) continue;
        g_configs[i].fired = true;
        if (g_configs[i].effect == Effect::ClearSelfStatus) Status = C4OS_DELETED;
        if (g_configs[i].effect == Effect::ClearRemovingStatus && g_removing)
            g_removing->Status = C4OS_DELETED;
        if (g_configs[i].effect == Effect::RemoveRemoving && g_removing)
            g_removing->AssignRemoval();
    }
    return 0;
}

void EffectsStub::ClearAll(C4Object *object, int32_t)
{
    trace("ClearAllEffects");
    if (Resurrects && object) object->Alive = 1;
}

// PSF_Death takes the death-causing player, which AssignDeath captures BEFORE
// the effect clear can meddle with it.
#define PSF_Death "~Death"

#include "object_assign_removal.inc"
#include "object_assign_death.inc"
} // namespace object_removal


// ---------------------------------------------------------------------------
// C4MouseControl::UpdateCursorTarget's OCF priority cascade (src/
// C4MouseControl.cpp). Every rule is an UNCONDITIONAL overwrite, so the LAST
// one that matches decides the cursor — not the first. An object that is at
// once carryable, choppable and alive walks the whole ladder and ends on the
// rule furthest down it.
//
// Only the cascade is lifted; the surrounding function handles regions,
// captions and drag state this section does not pin.
namespace mouse_cursor
{
struct C4Object;

// C4MouseControl.h's cursor ids, in the order the cascade assigns them.
const int32_t C4MC_Cursor_Crosshair = 0;
const int32_t C4MC_Cursor_Enter = 1;
const int32_t C4MC_Cursor_Grab = 2;
const int32_t C4MC_Cursor_Ungrab = 3;
const int32_t C4MC_Cursor_Object = 4;
const int32_t C4MC_Cursor_DigObject = 5;
const int32_t C4MC_Cursor_Chop = 6;
const int32_t C4MC_Cursor_Build = 7;
const int32_t C4MC_Cursor_Select = 8;
const int32_t C4MC_Cursor_Attack = 9;

// The OCF bits come from the real C4Constants.h the oracle already includes.
const int32_t C4D_MouseSelect = 1 << 22; // C4Def.h:73
const int32_t DFA_PUSH = 6;              // C4Def.h:436

struct ShapeStub
{
    int32_t Wdt{20};
};

struct ActionStub
{
    C4Object *Target{};
};

struct C4Object
{
    uint32_t OCF{};
    int32_t Category{};
    int32_t Owner{-1};
    bool Alive{};
    int32_t Procedure{-1};
    ShapeStub Shape;
    ActionStub Action;

    int32_t GetProcedure() const { return Procedure; }
    bool GetAlive() const { return Alive; }
};

struct PlayerStub
{
    C4Object *CrewMember{};

    bool ObjectInCrew(C4Object *object) const { return object && object == CrewMember; }
};

struct PlayerListStub
{
    PlayerStub *Held{};

    PlayerStub *Get(int32_t player) { return player >= 0 ? Held : nullptr; }
};

struct GameStub
{
    PlayerListStub Players;
};

static GameStub Game;

static bool ValidPlr(int32_t player) { return player >= 0; }

// Hostility is a player-relation question the cascade only consults; the
// fixture states the answer directly rather than modelling C4PlayerList.
static bool g_hostile = false;

static bool Hostile(int32_t, int32_t) { return g_hostile; }

// Run the lifted cascade against one candidate.
static int32_t run(
    uint32_t ocf, C4Object *TargetObject, C4Object *pPlrCursor, int32_t Player, int32_t X,
    int32_t Y, int32_t iObjX, int32_t iObjY)
{
    // The cascade's entry state: the default object cursor, set just above the
    // lifted fragment (C4MouseControl.cpp:478).
    int32_t Cursor = C4MC_Cursor_Crosshair;
#include "mouse_cursor_cascade.inc"
    return Cursor;
}
} // namespace mouse_cursor

// C4GameSave's save-policy matrix. The extracted query functions read only
// Sync, fInitial and the ctor flags, so the scaffold reproduces exactly those
// members: the out-of-line virtuals the real class also declares
// (AdjustCore/WriteDesc/SaveComponents/OnSaving) are never reached from a
// query, which is what lets this compile without linking the engine.
namespace game_save_policy
{

// SaveRuntimeData is an ordered sweep over component writers. Each writer is
// stubbed to a recorder: what is under test is which ones a policy reaches, in
// what order, and which failures abort — not what any component serialises.
static std::vector<std::string> g_trace;
static std::set<std::string> g_failing;

static bool wrote(const char *name)
{
	g_trace.emplace_back(name);
	return !g_failing.count(name);
}

enum class C4ResStrTableKey
{
	IDS_ERR_SAVE_SCENSECTIONS,
	IDS_ERR_SAVE_LANDSCAPE,
	IDS_ERR_SAVE_SCRIPTSTRINGS,
	IDS_ERR_SAVE_OBJECTS,
	IDS_ERR_ERRORSAVINGROUNDRESULTS,
	IDS_ERR_ERRORSAVINGTEAMS,
	IDS_ERR_SAVE_SCRIPT,
	IDS_ERR_SAVE_TITLE,
	IDS_ERR_SAVE_INFO,
	IDS_ERR_SAVE_RESTOREPLAYERINFOS,
	IDS_ERR_SAVE_PLAYERS,
};

// The log line is not the parity fact; that a failure was *reached* is, and
// the trace already carries it.
static void Log(C4ResStrTableKey) {}

struct C4Group
{
	void Delete(const char *name) { g_trace.emplace_back(std::string("delete:") + name); }
};

struct StringsStub
{
	void EnumStrings() { g_trace.emplace_back("EnumStrings"); }
	bool Save(C4Group &) { return wrote("Strings"); }
};

struct ScriptEngineStub { StringsStub Strings; };
struct ObjectsStub { bool Save(C4Group &, bool, bool) { return wrote("Objects"); } };
struct RoundResultsStub { bool Save(C4Group &) { return wrote("RoundResults"); } };
struct TeamsStub { bool Save(C4Group &) { return wrote("Teams"); } };
struct ScriptStub { bool Save(C4Group &) { return wrote("Script"); } };
struct TitleStub { bool Save(C4Group &) { return wrote("Title"); } };
struct InfoStub { bool Save(C4Group &) { return wrote("Info"); } };
struct PlayerInfosStub {};

struct C4PlayerInfoList
{
	void SetAsRestoreInfos(PlayerInfosStub &, bool user, bool script, bool user_files, bool script_files)
	{
		g_trace.emplace_back("SetAsRestoreInfos");
		(void)user; (void)script; (void)user_files; (void)script_files;
	}
	bool Save(C4Group &, const char *) { return wrote("RestorePlayerInfos"); }
};

struct PlayersStub
{
	bool Save(C4Group &, bool, C4PlayerInfoList &) { return wrote("Players"); }
};

struct GameStub
{
	ScriptEngineStub ScriptEngine;
	ObjectsStub Objects;
	RoundResultsStub RoundResults;
	TeamsStub Teams;
	ScriptStub Script;
	TitleStub Title;
	InfoStub Info;
	PlayerInfosStub PlayerInfos;
	PlayersStub Players;
};

static GameStub Game;

struct C4GameSave
{
	bool fInitial;

	enum SyncState
	{
		SyncNONE = 0,
		SyncScenario = 1,
		SyncSavegame = 2,
		SyncSynchronized = 3,
	} Sync;

	C4GameSave(bool fAInitial, SyncState ASync) : fInitial(fAInitial), Sync(ASync) {}
	virtual ~C4GameSave() = default;

	bool IsExact() { return Sync >= SyncSavegame; }
	bool IsSynced() { return Sync >= SyncSynchronized; }

	C4Group *pSaveGroup = nullptr;

	// The two fallible steps SaveRuntimeData opens with. Neither is under test
	// here — SaveLandscape needs a landscape and SaveScenarioSections a section
	// list — so both record and honour the injected failure set.
	bool SaveScenarioSections() { return wrote("ScenarioSections"); }
	bool SaveLandscape() { return wrote("Landscape"); }

	bool SaveRuntimeData();

#include "game_save_base_queries.inc"
};

struct C4GameSaveScenario : C4GameSave
{
	bool fForceExactLandscape;
	bool fSaveOrigin;

	C4GameSaveScenario(bool fForceExactLandscape, bool fSaveOrigin)
		: C4GameSave(false, SyncScenario),
		  fForceExactLandscape(fForceExactLandscape),
		  fSaveOrigin(fSaveOrigin) {}

#include "game_save_scenario_queries.inc"
};

struct C4GameSaveSavegame : C4GameSave
{
	C4GameSaveSavegame() : C4GameSave(false, SyncSavegame) {}

#include "game_save_savegame_queries.inc"
};

struct C4GameSaveRecord : C4GameSave
{
	bool fCopyScenario;

	C4GameSaveRecord(bool fAInitial, bool fACopyScenario)
		: C4GameSave(fAInitial, SyncSynchronized), fCopyScenario(fACopyScenario) {}

#include "game_save_record_queries.inc"
};

struct C4GameSaveNetwork : C4GameSave
{
	C4GameSaveNetwork(bool fAInitial) : C4GameSave(fAInitial, SyncSynchronized) {}

#include "game_save_network_queries.inc"
};

#include "game_save_runtime_data.inc"

// Read one variant's whole decision vector through the base pointer, so every
// value goes through the same virtual dispatch the real Save() call uses.
struct Vector
{
	bool save_runtime_data, keep_title, save_desc, copy_scenario, create_small_file;
	bool force_exact_landscape, save_origin, clear_origin;
	bool save_user_players, save_script_players;
	bool save_user_player_files, save_script_player_files;
	bool is_exact, is_synced;
	const char *sort_order;
};

inline Vector read(C4GameSave &save)
{
	Vector v;
	v.save_runtime_data = save.GetSaveRuntimeData();
	v.keep_title = save.GetKeepTitle();
	v.save_desc = save.GetSaveDesc();
	v.copy_scenario = save.GetCopyScenario();
	v.create_small_file = save.GetCreateSmallFile();
	v.force_exact_landscape = save.GetForceExactLandscape();
	v.save_origin = save.GetSaveOrigin();
	v.clear_origin = save.GetClearOrigin();
	v.save_user_players = save.GetSaveUserPlayers();
	v.save_script_players = save.GetSaveScriptPlayers();
	v.save_user_player_files = save.GetSaveUserPlayerFiles();
	v.save_script_player_files = save.GetSaveScriptPlayerFiles();
	v.is_exact = save.IsExact();
	v.is_synced = save.IsSynced();
	v.sort_order = save.GetSortOrder();
	return v;
}

} // namespace game_save_policy

// C4Group's entry matcher, lifted whole. Nothing but tolower is needed, so the
// real backtracking loop runs here rather than a restatement of it.
namespace wildcard
{

#include "wildcard_match.inc"

} // namespace wildcard

// The pure C4Strings helpers, lifted whole. They must sit at global scope so
// each definition matches the declaration in the real C4Strings.h -- that
// header is where the default arguments (SizeMax, ';', false) come from, and
// SAppend calls SCopy with two arguments relying on exactly that.
#include "c4strings_helpers.inc"
#include "c4strings_advance_space.inc"

// C4ConfigGeneral::GetLanguageSequence, with the smallest class that lets the
// real out-of-line definition compile.
namespace config_language
{

struct C4ConfigGeneral
{
	int GetLanguageSequence(const char *strSource, char *strTarget);
};

#include "config_language_sequence.inc"

} // namespace config_language

// C4Value::operator==, lifted whole. The scaffold supplies only what the
// operator itself touches: the tag enum, the C4V_Data union (compared as one
// word, and contextually convertible to bool for the `assert(!Data)` arms) and
// the three container types it dereferences. C4ValueList's real comparison is
// `= default` over a `std::vector<C4Value>` (C4ValueList.h:49,:67), so the
// array arm recurses back through this same operator element-wise.
namespace c4value_equal
{

enum C4V_Type
{
	C4V_Any = 0,
	C4V_Int,
	C4V_Bool,
	C4V_C4ID,
	C4V_C4Object,
	C4V_String,
	C4V_Array,
	C4V_Map,
	C4V_Ref,
};

struct C4Value;

struct C4String
{
	std::string Data;
};

struct C4ValueArray
{
	std::vector<C4Value> values;
	bool operator==(const C4ValueArray &other) const;
};

struct C4ValueMapData
{
	std::vector<std::pair<std::string, C4Value>> entries;
	bool operator==(const C4ValueMapData &other) const;
};

union C4V_Data
{
	std::intptr_t Int;
	C4String *Str;
	C4ValueArray *Array;
	C4ValueMapData *Map;

	bool operator==(const C4V_Data &other) const { return Int == other.Int; }
	explicit operator bool() const { return Int != 0; }
};

struct C4Value
{
	C4V_Data Data;
	C4V_Type Type;

	C4V_Data GetData() const { return Data; }
	bool operator==(const C4Value &Value2) const;
};

#include "c4value_operator_equal.inc"

inline bool C4ValueArray::operator==(const C4ValueArray &other) const
{
	return values == other.values;
}

inline bool C4ValueMapData::operator==(const C4ValueMapData &other) const
{
	return entries == other.entries;
}

inline C4Value scalar(C4V_Type type, std::intptr_t payload)
{
	C4Value value;
	value.Data.Int = payload;
	value.Type = type;
	return value;
}

inline C4Value string_value(C4String *str)
{
	C4Value value;
	value.Data.Str = str;
	value.Type = C4V_String;
	return value;
}

inline C4Value array_value(C4ValueArray *array)
{
	C4Value value;
	value.Data.Array = array;
	value.Type = C4V_Array;
	return value;
}

} // namespace c4value_equal

// ---------------------------------------------------------------------------
// C4PXS::Execute (src/C4PXS.cpp:28-135), the per-tick movement of one
// synchronized loose pixel. `pxs_allocation` covers only the allocator; this
// pins the step itself, which is on the bit-exact list: the raw C4Fixed
// position/velocity, the gravity accumulation, the airborne wind branch with
// its exact pair of Random(1200) draws, and the _PathFree fast path.
//
// Every case runs at zero wind. GBackWind is a constant here but the port
// answers it from the weather model (environment.wind_force(frame)), so a
// nonzero case would compare a stub against a simulation rather than the
// arithmetic under test. The draws and the WindDrift friction are exercised
// either way; a wind case belongs with a weather fixture.
//
// The fixture deliberately defines no material reactions, so
// GetReactionUnsafe answers nullptr and both reaction arms short-circuit. That
// isolates the movement and RNG rules from the reaction table.
//
// It also bounds what the section may cover: every case keeps the pixel in
// open sky. A pixel touching landscape material is exactly where the real
// reaction map decides — a liquid meeting denser ground is absorbed by the
// builtin Insert arm — so a stubbed lookup cannot model it. The contact arm
// needs a fixture carrying the reaction table and belongs in its own section.
namespace pxs_exec
{
const int32_t GridWdt = 16;
const int32_t GridHgt = 12;
const int32_t MNone = -1;

// 0 sky, 1 earth (solid), 2 water. Density/WindDrift mirror C4Material.h's
// C4M_Solid=50 and the shipped liquid values closely enough to drive the
// branch; the parity fact is the arithmetic, not the material table.
struct MaterialCore
{
    int32_t Density;
    int32_t WindDrift;
};

enum MaterialInteractionEvent
{
    meePXSPos = 0,
    meePXSMove = 1,
    meeMassMove = 2,
};

struct C4MaterialReaction;
using ReactionFunc = bool (*)(C4MaterialReaction *, int32_t &, int32_t &, int32_t,
                              int32_t, C4Fixed &, C4Fixed &, int32_t, int32_t,
                              MaterialInteractionEvent, bool *);
struct C4MaterialReaction
{
    ReactionFunc pFunc;
};

static int32_t g_grid[GridHgt][GridWdt];
static int32_t g_pix_cnt[GridWdt][GridHgt];
static int32_t g_wind = 0;

const int32_t GBackWdt = GridWdt;
const int32_t GBackHgt = GridHgt;

struct C4MaterialMap
{
    MaterialCore Map[3] = {{0, 0}, {50, 0}, {25, 40}};
    // No reaction is defined for this fixture, so every lookup answers null and
    // the lifted body takes its non-destructive paths.
    C4MaterialReaction *GetReactionUnsafe(int32_t, int32_t) { return nullptr; }
};

struct LandscapeStub
{
    C4Fixed Gravity{itofix(20, 100)};
    // C4Landscape::_PathFree is exactly C4LandscapePath::IsFree over PixCnt
    // (C4Landscape.cpp:891-897).
    bool _PathFree(int32_t x, int32_t y, int32_t x2, int32_t y2)
    {
        return C4LandscapePath::IsFree(x, y, x2, y2, [](int32_t cellX, int32_t cellY)
        {
            return g_pix_cnt[cellX][cellY] != 0;
        });
    }
};

struct GameStub
{
    int32_t FrameCounter = 0;
    C4MaterialMap Material;
    LandscapeStub Landscape;
};

static GameStub Game;

#define GravAccel (Game.Landscape.Gravity)
static const C4Fixed WindDrift_Factor = itofix(1, 800);

static FILE *LcRngTraceFile() { return nullptr; }
static bool MatValid(int32_t mat) { return mat >= 0 && mat < 3; }
static int32_t Sign(int32_t x) { return x < 0 ? -1 : (x > 0 ? 1 : 0); }

static int32_t GBackMat(int32_t x, int32_t y)
{
    if (x < 0 || x >= GridWdt || y < 0 || y >= GridHgt) return MNone;
    const int32_t mat = g_grid[y][x];
    return mat ? mat : MNone;
}

static int32_t GBackDensity(int32_t x, int32_t y)
{
    const int32_t mat = GBackMat(x, y);
    return mat < 0 ? 0 : Game.Material.Map[mat].Density;
}

static int32_t GBackWind(int32_t, int32_t) { return g_wind; }

struct C4PXS
{
    int32_t Mat{MNone};
    C4Fixed x, y, xdir, ydir;
    bool deactivated{false};

    void Execute();
    void Deactivate() { deactivated = true; Mat = MNone; }
};

#include "pxs_execute.inc"

} // namespace pxs_exec

// ---------------------------------------------------------------------------
// mrfInsertCheck (src/C4Material.cpp:567-609) with the C4Landscape::FindMatSlide
// it calls (src/C4Landscape.cpp:1247-1277). This is the arm every falling pixel
// takes on landing, and it is worth pinning for two reasons: its RNG ledger is
// property-dependent — a rough contact spends two draws on the splash roll, an
// incendiary material two more on its smoke, and a found slide one further —
// and it rewrites the pixel's position and velocity in place, so a wrong branch
// both moves the pixel and desynchronises the stream.
namespace insert_check
{
const int32_t GridWdt = 16;
const int32_t GridHgt = 12;
const int32_t MNone = -1;

struct MaterialCore
{
    int32_t Density;
    int32_t SplashRate;
    int32_t Incindiary;
    int32_t MaxSlide;
    int32_t InMatConvertDepth;
    int32_t InMatConvertTo;
    // `C4Landscape::Incinerate` reads this, not Incindiary: Incindiary is the
    // PXS's own smoke property, Inflammable is whether the landscape material
    // catches (C4Landscape.cpp:1478-1488).
    int32_t Inflammable;
};

enum MaterialInteractionEvent
{
    meePXSPos = 0,
    meePXSMove = 1,
    meeMassMove = 2,
};

struct C4MaterialReaction
{
    bool fUserDefined;
    bool fInsertionCheck;
    int32_t iExecMask;
    int32_t iDepth;
    int32_t iConvertMat;
};

static int32_t g_grid[GridHgt][GridWdt];
static int32_t g_smoke = 0;

struct C4MaterialMap
{
    // 0 vacuum, 1 water (splashes), 2 lava (incendiary), 3 granite (the floor).
    MaterialCore Map[4] = {
        {0, 0, 0, 0, 0, -1, 0},
        {25, 1, 0, 4, 0, -1, 0},  // Water: SplashRate 1 makes `!Random(1)` certain
        // Lava: incendiary (its own smoke) AND inflammable (the landscape
        // material catches), which are separate fields C4Landscape::Incinerate
        // and mrfInsertCheck read for different reasons.
        {25, 0, 1, 4, 2, 3, 1},
        {50, 0, 0, 0, 0, -1, 0},  // Granite floor
    };

    bool mrfConvert(C4MaterialReaction *pReaction, int32_t &iX, int32_t &iY,
                    int32_t iLSPosX, int32_t iLSPosY, C4Fixed &fXDir, C4Fixed &fYDir,
                    int32_t &iPxsMat, int32_t iLsMat, MaterialInteractionEvent evEvent,
                    bool *pfPosChanged);
    bool mrfInsert(C4MaterialReaction *pReaction, int32_t &iX, int32_t &iY,
                   int32_t iLSPosX, int32_t iLSPosY, C4Fixed &fXDir, C4Fixed &fYDir,
                   int32_t &iPxsMat, int32_t iLsMat, MaterialInteractionEvent evEvent,
                   bool *pfPosChanged);
    bool mrfIncinerate(C4MaterialReaction *pReaction, int32_t &iX, int32_t &iY,
                       int32_t iLSPosX, int32_t iLSPosY, C4Fixed &fXDir, C4Fixed &fYDir,
                       int32_t &iPxsMat, int32_t iLsMat, MaterialInteractionEvent evEvent,
                       bool *pfPosChanged);
    bool mrfPoof(C4MaterialReaction *pReaction, int32_t &iX, int32_t &iY,
                 int32_t iLSPosX, int32_t iLSPosY, C4Fixed &fXDir, C4Fixed &fYDir,
                 int32_t &iPxsMat, int32_t iLsMat, MaterialInteractionEvent evEvent,
                 bool *pfPosChanged);
};

// PXS creation is only reached by the MassMove arm; record it rather than
// simulating a PXS system.
static int32_t g_pxs_created = 0;
static int32_t g_pxs_created_mat = -1;
struct PxsStub
{
    // Record the material the mass-move arm hands to the PXS system: it is
    // the mover's ORIGINAL material, never the convert target, because the
    // meeMassMove case never reaches the reassignment above it.
    void Create(int32_t mat, C4Fixed, C4Fixed)
    {
        g_pxs_created++;
        g_pxs_created_mat = mat;
    }
};

// InsertMaterial is a whole landscape mutation of its own and deserves its own
// section; here it only has to record that mrfInsert reached it, and with what.
static int32_t g_inserted = 0;
static int32_t g_inserted_mat = -1, g_inserted_x = -1, g_inserted_y = -1;

// `C4Landscape::Incinerate` derives its answer from the landscape both engines
// read, so it is modelled rather than stubbed to a dictated result: a case that
// forced "it ignited" over a sky pixel would describe a state neither engine
// can reach. The one genuine input is whether a FLAM already stands in the
// 8x20 rect at (x-4, y-1), which C++ tests with FindObject and which suppresses
// the ignition (C4Landscape.cpp:1478-1488).
static int32_t g_incinerate_calls = 0;
static int32_t g_incinerate_x = -1, g_incinerate_y = -1;
static bool g_flam_already_here = false;
static int32_t g_flams_created = 0;

// mrfPoof's landscape side: it extracts the material it lands on. Recorded
// rather than performed, the same way InsertMaterial is.
static int32_t g_extractions = 0;
static int32_t g_extract_x = -1, g_extract_y = -1;
static int32_t g_sounds = 0;

struct C4Landscape
{
    C4Fixed Gravity{itofix(20, 100)};
    int32_t GetDensity(int32_t x, int32_t y);
    bool FindMatSlide(int32_t &fx, int32_t &fy, int32_t ydir, int32_t mdens, int32_t mslide);
    bool InsertMaterial(int32_t mat, int32_t tx, int32_t ty)
    {
        g_inserted++;
        g_inserted_mat = mat;
        g_inserted_x = tx;
        g_inserted_y = ty;
        return true;
    }
    // `C4Landscape::Incinerate` (C4Landscape.cpp:1478-1488) is a landscape
    // mutation of its own -- it reads GetMat, checks Inflammable, refuses when
    // a FLAM already stands in an 8x20 rect at (x-4, y-1), and only then
    // creates one. Its own decisions deserve their own section; here it only
    // has to record that mrfIncinerate reached it, and answer what the case
    // says it answered.
    bool Incinerate(int32_t x, int32_t y);
    int32_t ExtractMaterial(int32_t x, int32_t y)
    {
        g_extractions++;
        g_extract_x = x;
        g_extract_y = y;
        return 0;
    }
};

struct GameStub
{
    C4MaterialMap Material;
    C4Landscape Landscape;
    PxsStub PXS;
};

static GameStub Game;

#define GravAccel (Game.Landscape.Gravity)

static int32_t GBackMat(int32_t x, int32_t y)
{
    if (x < 0 || x >= GridWdt || y < 0 || y >= GridHgt) return 3; // closed border reads solid
    const int32_t mat = g_grid[y][x];
    return mat ? mat : MNone;
}

int32_t C4Landscape::GetDensity(int32_t x, int32_t y)
{
    const int32_t mat = GBackMat(x, y);
    return mat < 0 ? 0 : Game.Material.Map[mat].Density;
}

// `C4Landscape::Incinerate` (C4Landscape.cpp:1478-1488) in the shape
// mrfIncinerate depends on: the material at the pixel must be valid and
// inflammable, no FLAM may already stand in the rect, and the creation must
// succeed. It returns true only when it actually created one.
bool C4Landscape::Incinerate(int32_t x, int32_t y)
{
    g_incinerate_calls++;
    g_incinerate_x = x;
    g_incinerate_y = y;
    const int32_t mat = GBackMat(x, y);
    if (!MatValid(mat)) return false;
    if (!Game.Material.Map[mat].Inflammable) return false;
    if (g_flam_already_here) return false;
    g_flams_created++;
    return true;
}

#include "find_mat_slide.inc"

static void Smoke(int32_t, int32_t, int32_t) { g_smoke++; }
static void StartSoundEffectAt(const char *, int32_t, int32_t) { g_sounds++; }
static int32_t Sign(int32_t x) { return x < 0 ? -1 : (x > 0 ? 1 : 0); }
template <class T> static T Abs(T v) { return v < 0 ? -v : v; }

static bool MatValid(int32_t mat) { return mat >= 0 && mat < 4; }

#include "mrf_insert_check.inc"
#include "mrf_user_check.inc"
#include "mrf_convert.inc"
#include "mrf_insert.inc"
#include "mrf_incinerate.inc"
#include "mrf_poof.inc"

} // namespace insert_check

// Runs one mrfInsertCheck and records the rewritten position/velocity, the
// verdict, and the RNG ledger. The draw count is the point: it varies with the
// material's SplashRate and Incindiary and with whether a slide was found.
// mrfConvert's two easily-lost rules: C++'s `case meePXSMove:` falls THROUGH
// into `meePXSPos` when the reaction is user-defined, and a *successful*
// conversion returns false ("not handled") while a conversion to an unloaded or
// sky target returns true and kills the pixel.
// mrfInsert's splash/slide check is `!fUserDefined`-gated INSIDE the movement
// case (C4Material.cpp:783-787), because a user-defined reaction already ran
// the same check through mrfUserCheck. Dropping that gate runs the check twice
// and doubles the synchronized draws on every inserting pixel, which is why the
// draw count is emitted alongside the verdict.
static void printInsertCases()
{
    printf("\"insert_arm\":[");
    struct Case
    {
        const char *name;
        bool user_defined;
        bool insertion_check;  // CheckSlide=
        int32_t event;
        int32_t pxs_mat;
        int32_t ydir_n, ydir_p;
    };
    const Case cases[] = {
        // Only the movement event inserts; the other two break straight out.
        {"pos_event_unhandled", false, true, 0 /*meePXSPos*/, 1, 1, 2},
        {"mass_move_unhandled", false, true, 2 /*meeMassMove*/, 1, 1, 2},
        // Rough contact on a splashing material: the check refuses, so the
        // pixel keeps existing and nothing is inserted. Two draws.
        {"hardcoded_splash_blocks_insert", false, true, 1 /*meePXSMove*/, 1, 3, 1},
        // Incendiary contact passes the check but spends one draw on the way.
        {"hardcoded_incendiary_inserts", false, true, 1, 2, 1, 2},
        // The same incendiary insertion as a USER reaction: the check runs once,
        // in mrfUserCheck, and the body's own call is gated off. Still one draw
        // — two would mean the gate was lost.
        {"user_insert_checks_once", true, true, 1, 2, 1, 2},
        // CheckSlide=0 skips the check entirely, so a rough splash contact
        // inserts anyway and spends nothing.
        {"user_no_check_inserts", true, false, 1, 1, 3, 1},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        // Boxed in over a solid floor, so FindMatSlide has no target and the
        // check's verdict is decided by the splash arm alone.
        for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                insert_check::g_grid[gy][gx] = (gx == 8) ? 0 : 3;
        for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
            insert_check::g_grid[10][gx] = 3;
        insert_check::g_smoke = 0;
        insert_check::g_inserted = 0;
        insert_check::g_inserted_mat = -1;
        insert_check::g_inserted_x = -1;
        insert_check::g_inserted_y = -1;

        const int32_t seed = 0x2222;
        FixedRandom(seed);
        Randomize3();
        const int32_t draws_before = RandomCount;

        insert_check::C4MaterialReaction reaction{};
        reaction.fUserDefined = c.user_defined;
        reaction.fInsertionCheck = c.insertion_check;
        reaction.iExecMask = ~0;
        reaction.iDepth = 0;
        reaction.iConvertMat = 0;

        const int32_t ls_mat = 3;
        int32_t iX = 8, iY = 9;
        C4Fixed xdir = itofix(0, 1), ydir = itofix(c.ydir_n, c.ydir_p);
        int32_t pxs_mat = c.pxs_mat;
        bool pos_changed = false;
        const bool handled = insert_check::Game.Material.mrfInsert(
            &reaction, iX, iY, iX, iY, xdir, ydir, pxs_mat, ls_mat,
            static_cast<insert_check::MaterialInteractionEvent>(c.event), &pos_changed);

        printf("{\"name\":\"%s\",\"user_defined\":%s,\"insertion_check\":%s,"
               "\"event\":%d,\"pxs_mat\":%d,\"ls_mat\":%d,\"x0\":%d,\"y0\":%d,"
               "\"xdir0\":%d,\"ydir0\":%d,\"seed\":%d,\"handled\":%s,\"x\":%d,"
               "\"y\":%d,\"xdir\":%d,\"ydir\":%d,\"pos_changed\":%s,\"draws\":%d,"
               "\"inserted\":%d,\"inserted_mat\":%d,\"inserted_x\":%d,"
               "\"inserted_y\":%d}",
               c.name, c.user_defined ? "true" : "false",
               c.insertion_check ? "true" : "false", c.event, c.pxs_mat, ls_mat,
               8, 9, itofix(0, 1).val, itofix(c.ydir_n, c.ydir_p).val, seed,
               handled ? "true" : "false", iX, iY, xdir.val, ydir.val,
               pos_changed ? "true" : "false", RandomCount - draws_before,
               insert_check::g_inserted, insert_check::g_inserted_mat,
               insert_check::g_inserted_x, insert_check::g_inserted_y);
    }
    printf("]");
}

// --- mrfPoof's movement arm -------------------------------------------------
//
// `material_poof_reaction` above pins the two Rnd3 draws, but it covers only
// the two non-movement events and never runs the reaction itself -- its rows
// are all `handled: 1`, so the unhandled outcome has never been exercised.
//
// `meePXSMove` is where the unhandled outcome lives, and the existing section
// deliberately avoids it because it walks the landscape. This scaffold already
// has that landscape, so the arm can run here:
//
//   * a non-user reaction runs `mrfInsertCheck` FIRST, and a splash that
//     prevents the interaction returns unhandled having extracted **nothing**
//     and drawn **nothing** -- the draws are downstream of the check, so a port
//     that extracted or drew before checking would desynchronise;
//   * a user-defined reaction skips that check, because `mrfUserCheck` already
//     ran it, and extracts anyway.
static void printPoofMoveCases()
{
    printf("\"poof_arm\":[");
    struct Case
    {
        const char *name;
        bool user_defined;
        int32_t event;
        int32_t pxs_mat;
        int32_t ydir_n, ydir_p;
    };
    const Case cases[] = {
        // Smooth contact: the check passes, so the arm extracts and draws.
        {"move_extracts_after_check", false, 1 /*meePXSMove*/, 2, 1, 2},
        // Rough splashing contact: the check refuses BEFORE anything is
        // extracted or drawn.
        {"move_check_blocks_before_extracting", false, 1, 1, 3, 1},
        // A user reaction runs the check in mrfUserCheck INSTEAD, at the top of
        // the function, so the same rough contact is refused there and never
        // reaches the body at all.
        {"user_move_blocked_by_the_user_check", true, 1, 1, 3, 1},
        // The same user reaction on a smooth contact: mrfUserCheck passes and
        // the body's own call is gated off, so this spends ONE check's draws,
        // not two. Two would mean the gate was lost.
        {"user_move_checks_once", true, 1, 2, 1, 2},
        // The non-movement arms for comparison: always handled.
        {"pos_extracts", false, 0 /*meePXSPos*/, 2, 1, 2},
        {"mass_move_extracts", false, 2 /*meeMassMove*/, 2, 1, 2},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                insert_check::g_grid[gy][gx] = (gx == 8) ? 0 : 3;
        for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
            insert_check::g_grid[10][gx] = 3;
        insert_check::g_smoke = 0;
        insert_check::g_sounds = 0;
        insert_check::g_extractions = 0;
        insert_check::g_extract_x = -1;
        insert_check::g_extract_y = -1;
        insert_check::g_inserted = 0;

        const int32_t seed = 0x2222;
        FixedRandom(seed);
        Randomize3();
        const int32_t draws_before = RandomCount;

        insert_check::C4MaterialReaction reaction{};
        reaction.fUserDefined = c.user_defined;
        reaction.fInsertionCheck = true;
        reaction.iExecMask = ~0;
        reaction.iDepth = 0;
        reaction.iConvertMat = 0;

        const int32_t ls_mat = 3;
        int32_t iX = 8, iY = 9;
        C4Fixed xdir = itofix(0, 1), ydir = itofix(c.ydir_n, c.ydir_p);
        int32_t pxs_mat = c.pxs_mat;
        bool pos_changed = false;
        const bool handled = insert_check::Game.Material.mrfPoof(
            &reaction, iX, iY, iX, iY, xdir, ydir, pxs_mat, ls_mat,
            static_cast<insert_check::MaterialInteractionEvent>(c.event), &pos_changed);

        printf("{\"name\":\"%s\",\"user_defined\":%s,\"event\":%d,\"pxs_mat\":%d,"
               "\"ls_mat\":%d,\"x0\":%d,\"y0\":%d,\"xdir0\":%d,\"ydir0\":%d,"
               "\"seed\":%d,\"handled\":%s,\"x\":%d,\"y\":%d,\"xdir\":%d,"
               "\"ydir\":%d,\"pos_changed\":%s,\"draws\":%d,\"extractions\":%d,"
               "\"extract_x\":%d,\"extract_y\":%d}",
               c.name, c.user_defined ? "true" : "false", c.event, c.pxs_mat, ls_mat,
               8, 9, itofix(0, 1).val, itofix(c.ydir_n, c.ydir_p).val, seed,
               handled ? "true" : "false", iX, iY, xdir.val, ydir.val,
               pos_changed ? "true" : "false", RandomCount - draws_before,
               insert_check::g_extractions, insert_check::g_extract_x,
               insert_check::g_extract_y);
    }
    printf("]");
}

// --- mrfIncinerate ----------------------------------------------------------
//
// The three arms are asymmetric, and flattening them is the likely port error:
//
//   * `meeMassMove` and `meePXSPos` try to incinerate and report **unhandled**
//     when the pixel does not ignite. Unhandled means the caller keeps looking,
//     so answering "handled" there silently swallows the pixel.
//   * `meePXSMove` runs the insertion check FIRST. A splash or slide that
//     prevents the interaction returns unhandled *before anything burns*, so a
//     port that incinerates first would ignite pixels C++ never touches.
//   * Only `meePXSMove` inserts the pixel when it fails to ignite. The other
//     two drop it.
//
// The switch has no default arm, so any event outside those three never reaches
// `C4Landscape::Incinerate` at all -- the call count pins that.
//
// Every row's ignition is DERIVED from the fixture, never dictated: the target
// pixel is inflammable or it is not, and the one separate input is whether a
// FLAM already stands in the 8x20 rect at (x-4, y-1) that suppresses a second
// one. A row that forced "it ignited" over a sky pixel would describe a state
// neither engine can reach.
static void printIncinerateCases()
{
    printf("\"incinerate_arm\":[");
    struct Case
    {
        const char *name;
        int32_t event;
        int32_t target_mat;   // what sits at the target pixel
        bool flam_here;       // a FLAM already occupies the rect
        int32_t pxs_mat;
        int32_t ydir_n, ydir_p;
    };
    const Case cases[] = {
        // The two non-movement arms never run the insertion check, so the
        // target may be any material without disturbing anything else.
        {"pos_ignites_inflammable", 0 /*meePXSPos*/, 2, false, 1, 1, 2},
        {"pos_sky_does_not_ignite", 0, 0, false, 1, 1, 2},
        {"pos_suppressed_by_existing_flam", 0, 2, true, 1, 1, 2},
        {"mass_move_ignites_inflammable", 2 /*meeMassMove*/, 2, false, 1, 1, 2},
        {"mass_move_sky_does_not_ignite", 2, 0, false, 1, 1, 2},
        // Movement over sky: the check passes, nothing ignites, and the pixel
        // is INSERTED rather than dropped.
        {"move_dead_inserts", 1 /*meePXSMove*/, 0, false, 1, 1, 2},
        // Movement on a rough splashing contact: the check refuses, so the
        // landscape is never asked to incinerate at all.
        {"move_check_blocks_before_burning", 1, 0, false, 1, 3, 1},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        // Same boxed-in fixture the insert arm uses, so the insertion check's
        // verdict is decided by the splash arm alone; the target pixel is then
        // set to whatever this row is about.
        for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                insert_check::g_grid[gy][gx] = (gx == 8) ? 0 : 3;
        for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
            insert_check::g_grid[10][gx] = 3;
        insert_check::g_grid[9][8] = c.target_mat;
        insert_check::g_smoke = 0;
        insert_check::g_inserted = 0;
        insert_check::g_inserted_mat = -1;
        insert_check::g_inserted_x = -1;
        insert_check::g_inserted_y = -1;
        insert_check::g_incinerate_calls = 0;
        insert_check::g_incinerate_x = -1;
        insert_check::g_incinerate_y = -1;
        insert_check::g_flam_already_here = c.flam_here;
        insert_check::g_flams_created = 0;

        const int32_t seed = 0x2222;
        FixedRandom(seed);
        Randomize3();
        const int32_t draws_before = RandomCount;

        // mrfIncinerate asserts !fUserDefined: it is not available as a user
        // reaction, so there is no user-defined row to record.
        insert_check::C4MaterialReaction reaction{};
        reaction.fUserDefined = false;
        reaction.fInsertionCheck = true;
        reaction.iExecMask = ~0;
        reaction.iDepth = 0;
        reaction.iConvertMat = 0;

        const int32_t ls_mat = 3;
        int32_t iX = 8, iY = 9;
        C4Fixed xdir = itofix(0, 1), ydir = itofix(c.ydir_n, c.ydir_p);
        int32_t pxs_mat = c.pxs_mat;
        bool pos_changed = false;
        const bool handled = insert_check::Game.Material.mrfIncinerate(
            &reaction, iX, iY, iX, iY, xdir, ydir, pxs_mat, ls_mat,
            static_cast<insert_check::MaterialInteractionEvent>(c.event), &pos_changed);

        printf("{\"name\":\"%s\",\"event\":%d,\"target_mat\":%d,\"flam_here\":%s,"
               "\"pxs_mat\":%d,\"ls_mat\":%d,\"x0\":%d,\"y0\":%d,\"xdir0\":%d,"
               "\"ydir0\":%d,\"seed\":%d,\"handled\":%s,\"x\":%d,\"y\":%d,"
               "\"xdir\":%d,\"ydir\":%d,\"pos_changed\":%s,\"draws\":%d,"
               "\"incinerate_calls\":%d,\"flams_created\":%d,\"inserted\":%d,"
               "\"inserted_mat\":%d,\"inserted_x\":%d,\"inserted_y\":%d}",
               c.name, c.event, c.target_mat, c.flam_here ? "true" : "false",
               c.pxs_mat, ls_mat, 8, 9, itofix(0, 1).val,
               itofix(c.ydir_n, c.ydir_p).val, seed, handled ? "true" : "false",
               iX, iY, xdir.val, ydir.val, pos_changed ? "true" : "false",
               RandomCount - draws_before, insert_check::g_incinerate_calls,
               insert_check::g_flams_created, insert_check::g_inserted,
               insert_check::g_inserted_mat, insert_check::g_inserted_x,
               insert_check::g_inserted_y);
    }
    printf("]");
}

// --- PXS casting into slots -------------------------------------------------
//
// `pxs_allocation` above already pins `C4PXSSystem::New`'s slot choice in
// isolation, over four slots freed out of order. This section runs the layer
// that USES it, and covers three things that one does not:
//
//   * **`Cast`'s draw order.** The C++ pulls both randoms into named locals
//     with a `// force argument evaluation order` comment, and the one drawn
//     FIRST (`r2`) is the one used for ydir. Reading them in argument order
//     swaps the two velocities while drawing exactly as many numbers, so no
//     draw-count check can see it — only the raw fixed values can.
//   * **Per-slot state and chunk counts**, rather than just which slot a
//     pointer landed in: `Create`'s field init is compared alongside it.
//   * **The chunk boundary.** Filling chunk 0 to its 500-slot capacity and
//     creating one more allocates chunk 1 on demand; `pxs_allocation` never
//     makes enough particles to reach it.
namespace pxs_slots
{
const int32_t MNone = -1;
const size_t PXSChunkSize = 500, PXSMaxChunk = 20;

static bool MatValid(int32_t mat) { return mat >= 0 && mat < 4; }

struct C4PXS
{
    int32_t Mat = MNone;
    C4Fixed x = itofix(0), y = itofix(0), xdir = itofix(0), ydir = itofix(0);
    void Deactivate();
};

struct C4Group;

struct C4PXSSystem
{
    int32_t Count = 0;
    C4PXS *Chunk[PXSMaxChunk] = {};
    size_t iChunkPXS[PXSMaxChunk] = {};

    C4PXS *New();
    bool Create(int32_t mat, C4Fixed ix, C4Fixed iy, C4Fixed ixdir, C4Fixed iydir);
    void Cast(int32_t mat, int32_t num, int32_t tx, int32_t ty, int32_t level);
    void Delete(C4PXS *pPXS);
    void Clear();
    bool Load(C4Group &hGroup);

    void reset()
    {
        for (size_t cnt = 0; cnt < PXSMaxChunk; cnt++)
        {
            delete[] Chunk[cnt];
            Chunk[cnt] = nullptr;
            iChunkPXS[cnt] = 0;
        }
        Count = 0;
    }
};

struct GameStub
{
    C4PXSSystem PXS;
};

static GameStub Game;

// `Load` reads its bytes through a C4Group. Only the two calls it makes are
// modelled, over a buffer the fixture fills — the group layer itself is not
// under test here, the length arithmetic and the per-slot recount are.
// C4CFN_PXS comes from the real C4Components.h, which is already included.
static std::vector<uint8_t> g_entry;
static size_t g_entry_pos = 0;
static bool g_entry_present = true;

struct C4Group
{
    bool AccessEntry(const char *, size_t *size)
    {
        if (!g_entry_present) return false;
        *size = g_entry.size();
        g_entry_pos = 0;
        return true;
    }

    bool Read(void *dest, size_t size)
    {
        if (g_entry_pos + size > g_entry.size()) return false;
        std::memcpy(dest, g_entry.data() + g_entry_pos, size);
        g_entry_pos += size;
        return true;
    }
};

#include "pxs_new.inc"
#include "pxs_create.inc"
#include "pxs_cast.inc"
#include "pxs_delete.inc"
#include "pxs_deactivate.inc"
#include "pxs_clear.inc"
#include "pxs_load.inc"
} // namespace pxs_slots

// `C4PXSSystem::Load`'s accept/reject decision is pure arithmetic on the file
// length: a four-byte number-format tag is detected by the remainder being
// exactly 4, NOT by reading a magic value, so a file whose payload happens to
// be four bytes past a chunk boundary is read as tagged. Everything else — the
// 1..2 format range, the chunk ceiling, the per-chunk recount — follows from
// that first decision, and the float conversion is applied only to slots whose
// material is set.
//
// The golden carries a compact recipe rather than the bytes (one case is 21
// chunks, 210 KB); both sides build the buffer from it.
struct PxsLoadSlot
{
    int32_t chunk, slot, mat, x, y, xdir, ydir;
};

struct PxsLoadCase
{
    const char *name;
    bool present;      // does the group hold a PXS entry at all
    int32_t tag;       // number-format tag, or 0 for "write no tag"
    int32_t chunks;    // whole chunks of payload
    int32_t extra;     // stray bytes appended, to break the length arithmetic
    std::vector<PxsLoadSlot> live;
};

static void buildPxsLoadEntry(const PxsLoadCase &c)
{
    pxs_slots::g_entry.clear();
    pxs_slots::g_entry_present = c.present;
    const auto push = [](int32_t value)
    {
        for (int b = 0; b < 4; b++)
            pxs_slots::g_entry.push_back(static_cast<uint8_t>((value >> (8 * b)) & 0xff));
    };
    if (c.tag) push(c.tag);
    const size_t payload_start = pxs_slots::g_entry.size();
    for (int32_t chunk = 0; chunk < c.chunks; chunk++)
        for (size_t slot = 0; slot < pxs_slots::PXSChunkSize; slot++)
        {
            push(pxs_slots::MNone);
            push(0); push(0); push(0); push(0);
        }
    for (const auto &live : c.live)
    {
        const size_t offset = payload_start
            + (static_cast<size_t>(live.chunk) * pxs_slots::PXSChunkSize
               + static_cast<size_t>(live.slot)) * 20;
        const int32_t fields[5] = {live.mat, live.x, live.y, live.xdir, live.ydir};
        for (int f = 0; f < 5; f++)
            for (int b = 0; b < 4; b++)
                pxs_slots::g_entry[offset + f * 4 + b] =
                    static_cast<uint8_t>((fields[f] >> (8 * b)) & 0xff);
    }
    for (int32_t b = 0; b < c.extra; b++)
        pxs_slots::g_entry.push_back(0);
}

static void printPxsLoadCases()
{
    printf("\"pxs_load\":[");
    // A float bit pattern, for the format-2 conversion: 2.5f and -0.75f.
    const int32_t f2_5 = 0x40200000, fm0_75 = 0xbf400000;
    const std::vector<PxsLoadCase> cases = {
        // No entry in the group at all: refused before anything is cleared.
        {"absent_entry", false, 0, 0, 0, {}},
        // A length that is neither a whole number of chunks nor four past one.
        {"ragged_length", true, 0, 1, 7, {}},
        // Exactly one chunk and no tag: the legacy untagged form, format 1.
        {"untagged_chunk", true, 0, 1, 0, {{0, 3, 2, 100, 200, 300, 400}}},
        // The same payload with a tag: detected by the remainder alone.
        {"tagged_chunk", true, 1, 1, 0, {{0, 3, 2, 100, 200, 300, 400}}},
        // Tag outside 1..2 is refused.
        {"bad_number_format", true, 3, 1, 0, {}},
        // Exactly at the chunk ceiling: accepted. The pair with the case
        // below is what separates `>` from `>=`.
        {"at_chunk_ceiling", true, 1, 20, 0, {{19, 0, 1, 41, 42, 43, 44}}},
        // One past it: refused.
        {"too_many_chunks", true, 1, 21, 0, {}},
        // Untagged, but the first payload word is a material id that is also a
        // valid format tag. Detecting the tag by VALUE rather than by the
        // length remainder mistakes this for a tagged file and shifts the whole
        // payload four bytes.
        {"untagged_first_slot_looks_like_a_tag", true, 0, 1, 0,
         {{0, 0, 2, 55, 66, 77, 88}}},
        // Two chunks, live slots in both, so the per-chunk recount has to
        // attribute them separately.
        {"recount_per_chunk", true, 1, 2, 0,
         {{0, 0, 1, 11, 12, 13, 14}, {0, 499, 2, 21, 22, 23, 24}, {1, 7, 3, 31, 32, 33, 34}}},
        // Format 2 stores floats. The live slot is converted; the dead slot
        // beside it keeps its raw bits, because the conversion is inside the
        // `Mat != MNone` branch.
        {"float_format", true, 2, 1, 0,
         {{0, 5, 1, f2_5, fm0_75, f2_5, fm0_75}}},
    };

    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        buildPxsLoadEntry(c);
        pxs_slots::Game.PXS.Clear();
        pxs_slots::C4Group group;
        const bool ok = pxs_slots::Game.PXS.Load(group);

        printf("{\"name\":\"%s\",\"present\":%s,\"tag\":%d,\"chunks\":%d,\"extra\":%d,"
               "\"input\":[",
               c.name, c.present ? "true" : "false", c.tag, c.chunks, c.extra);
        bool first_in = true;
        for (const auto &live : c.live)
        {
            if (!first_in) printf(",");
            first_in = false;
            printf("{\"chunk\":%d,\"slot\":%d,\"mat\":%d,\"x\":%d,\"y\":%d,"
                   "\"xdir\":%d,\"ydir\":%d}",
                   live.chunk, live.slot, live.mat, live.x, live.y, live.xdir, live.ydir);
        }
        printf("],\"ok\":%s,\"counts\":[", ok ? "true" : "false");
        for (size_t chunk = 0; chunk < 3; chunk++)
        {
            if (chunk) printf(",");
            printf("%zu", pxs_slots::Game.PXS.iChunkPXS[chunk]);
        }
        printf("],\"loaded\":[");
        bool first_out = true;
        for (const auto &live : c.live)
        {
            if (!ok) break;
            const pxs_slots::C4PXS *chunk = pxs_slots::Game.PXS.Chunk[live.chunk];
            if (!chunk) continue;
            const pxs_slots::C4PXS &pxp = chunk[live.slot];
            if (pxp.Mat == pxs_slots::MNone) continue;
            if (!first_out) printf(",");
            first_out = false;
            printf("{\"chunk\":%d,\"slot\":%d,\"mat\":%d,\"x\":%d,\"y\":%d,"
                   "\"xdir\":%d,\"ydir\":%d}",
                   live.chunk, live.slot, pxp.Mat, pxp.x.val, pxp.y.val,
                   pxp.xdir.val, pxp.ydir.val);
        }
        printf("]}");
    }
    pxs_slots::Game.PXS.Clear();
    pxs_slots::g_entry.clear();
    pxs_slots::g_entry_present = true;
    printf("]");
}

static void printPxsSlotStep(const char *step, int32_t draws)
{
    size_t live = 0;
    for (size_t cnt = 0; cnt < pxs_slots::PXSMaxChunk; cnt++)
        live += pxs_slots::Game.PXS.iChunkPXS[cnt];

    printf("{\"step\":\"%s\",\"draws\":%d,\"live\":%zu,\"chunks\":[", step, draws, live);
    for (size_t cnt = 0; cnt < 3; cnt++)
    {
        if (cnt) printf(",");
        printf("{\"i\":%zu,\"alloc\":%s,\"count\":%zu}", cnt,
               pxs_slots::Game.PXS.Chunk[cnt] ? "true" : "false",
               pxs_slots::Game.PXS.iChunkPXS[cnt]);
    }
    printf("],\"slots\":[");
    for (size_t slot = 0; slot < 6; slot++)
    {
        if (slot) printf(",");
        const pxs_slots::C4PXS *pxp =
            pxs_slots::Game.PXS.Chunk[0] ? &pxs_slots::Game.PXS.Chunk[0][slot] : nullptr;
        if (!pxp)
        {
            printf("{\"i\":%zu,\"mat\":%d,\"x\":0,\"y\":0,\"xdir\":0,\"ydir\":0}",
                   slot, pxs_slots::MNone);
            continue;
        }
        printf("{\"i\":%zu,\"mat\":%d,\"x\":%d,\"y\":%d,\"xdir\":%d,\"ydir\":%d}",
               slot, pxp->Mat, pxp->x.val, pxp->y.val, pxp->xdir.val, pxp->ydir.val);
    }
    printf("]}");
}

static void printPxsSlotCases()
{
    printf("\"pxs_slots\":[");
    pxs_slots::Game.PXS.reset();
    FixedRandom(0x5151);
    Randomize3();
    int32_t mark = RandomCount;

    // 1. Three cast particles take slots 0, 1, 2 of a freshly allocated chunk,
    //    and their velocities record which random went where.
    pxs_slots::Game.PXS.Cast(2, 3, 30, 40, 20);
    printPxsSlotStep("cast_three", RandomCount - mark);
    mark = RandomCount;

    // 2. Deactivating the middle particle frees slot 1 and decrements the
    //    chunk counter without touching its neighbours.
    printf(",");
    pxs_slots::Game.PXS.Chunk[0][1].Deactivate();
    printPxsSlotStep("free_middle", RandomCount - mark);
    mark = RandomCount;

    // 3. The next particle must land back in slot 1 — the freed one — rather
    //    than at the end.
    printf(",");
    pxs_slots::Game.PXS.Cast(1, 1, 10, 12, 4);
    printPxsSlotStep("reuse_freed_slot", RandomCount - mark);
    mark = RandomCount;

    // 4. Fill chunk 0 exactly to capacity. Nothing is drawn: Create is not Cast.
    printf(",");
    while (pxs_slots::Game.PXS.iChunkPXS[0] < pxs_slots::PXSChunkSize)
        pxs_slots::Game.PXS.Create(3, itofix(1), itofix(2), itofix(0), itofix(0));
    printPxsSlotStep("fill_chunk", RandomCount - mark);
    mark = RandomCount;

    // 5. One more spills into chunk 1, which is allocated on demand.
    printf(",");
    pxs_slots::Game.PXS.Create(3, itofix(7), itofix(8), itofix(0), itofix(0));
    printPxsSlotStep("spill_to_chunk1", RandomCount - mark);


    pxs_slots::Game.PXS.reset();
    printf("]");
}

static void printConvertCases()
{
    printf("\"convert_check\":[");
    struct Case
    {
        const char *name;
        bool user_defined;
        int32_t depth;        // user-defined depth; hardcoded reads the material
        int32_t convert_mat;  // user-defined target
        int32_t event;
        int32_t pxs_mat;
        int32_t ls_mat;
        bool matching_above;  // put ls_mat at (x, y - depth)
    };
    const Case cases[] = {
        // Hardcoded conversion has no collision proc, so a move event breaks
        // out before the depth check ever runs.
        {"hardcoded_move_breaks", false, 0, 0, 1 /*meePXSMove*/, 2, 3, true},
        // Same reaction at the position event converts: Lava -> Granite at
        // depth 2, with the matching material above.
        {"hardcoded_pos_converts", false, 0, 0, 0 /*meePXSPos*/, 2, 3, true},
        // Depth unsatisfied: nothing above matches, so no conversion.
        {"depth_unsatisfied", false, 0, 0, 0, 2, 3, false},
        // A user-defined reaction DOES convert on a move event — the C++
        // fallthrough.
        {"user_move_falls_through", true, 0, 3, 1, 1, 3, true},
        // Converting to an invalid target kills the pixel.
        {"invalid_target_kills", true, 0, 99, 0, 1, 3, true},
        // MassMove transfers the mover's own material to PXS and reports
        // handled. Hardcoded, so the convert target is never consulted.
        {"mass_move_creates_pxs", false, 0, 0, 2 /*meeMassMove*/, 2, 3, true},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                insert_check::g_grid[gy][gx] = 0;
        const int32_t px = 8, py = 6;
        const int32_t depth = c.user_defined
            ? c.depth
            : insert_check::Game.Material.Map[c.pxs_mat].InMatConvertDepth;
        if (c.matching_above && depth)
            insert_check::g_grid[py - depth][px] = c.ls_mat;
        insert_check::g_pxs_created = 0;
        insert_check::g_pxs_created_mat = -1;

        insert_check::C4MaterialReaction reaction{};
        reaction.fUserDefined = c.user_defined;
        reaction.fInsertionCheck = false;
        reaction.iExecMask = ~0;
        reaction.iDepth = c.depth;
        reaction.iConvertMat = c.convert_mat;

        int32_t iX = px, iY = py;
        C4Fixed xdir = itofix(1, 2), ydir = itofix(1, 2);
        int32_t pxs_mat = c.pxs_mat;
        bool pos_changed = false;
        const bool handled = insert_check::Game.Material.mrfConvert(
            &reaction, iX, iY, iX, iY, xdir, ydir, pxs_mat, c.ls_mat,
            static_cast<insert_check::MaterialInteractionEvent>(c.event), &pos_changed);

        printf("{\"name\":\"%s\",\"user_defined\":%s,\"depth\":%d,\"convert_mat\":%d,"
               "\"event\":%d,\"pxs_mat0\":%d,\"ls_mat\":%d,\"matching_above\":%s,"
               "\"handled\":%s,\"pxs_mat\":%d,\"xdir\":%d,\"ydir\":%d,"
               "\"pos_changed\":%s,\"pxs_created\":%d,\"pxs_created_mat\":%d}",
               c.name, c.user_defined ? "true" : "false", c.depth, c.convert_mat,
               c.event, c.pxs_mat, c.ls_mat, c.matching_above ? "true" : "false",
               handled ? "true" : "false", pxs_mat, xdir.val, ydir.val,
               pos_changed ? "true" : "false", insert_check::g_pxs_created,
               insert_check::g_pxs_created_mat);
    }
    printf("]");
}

static void printInsertCheckCases()
{
    printf("\"insert_check\":[");
    struct Case
    {
        const char *name;
        int32_t pxs_mat;
        int32_t ls_mat;
        int32_t x, y;
        int32_t xdir_n, xdir_p, ydir_n, ydir_p;
        int32_t seed;
        bool floor;      // solid row under the pixel
        bool walled;     // solid to both sides, so no slide exists
        int32_t hole;    // column where the floor is missing, or -1
    };
    const Case cases[] = {
        // Gentle contact, boxed in: no splash roll, no slide, insertion OK.
        {"insert_ok", 1, 3, 8, 9, 0, 1, 1, 2, 0x2222, true, true, -1},
        // Rough contact (fYDir > itofix(1)) on a splashing material. SplashRate
        // 1 makes the roll certain, so this pins the two-draw splash arm and
        // its `fYDir = -fYDir / 8` bounce rather than sampling it.
        {"splash", 1, 3, 8, 9, 0, 1, 3, 1, 0x2222, true, true, -1},
        // Incendiary contact rolls Random(25) before sliding.
        {"incendiary", 2, 3, 8, 9, 0, 1, 1, 2, 0x2222, true, true, -1},
        // A hole in the floor gives FindMatSlide a reachable target. Different
        // material, so the acceleration arm runs and spends one draw.
        {"slide_other_mat", 1, 3, 8, 9, 0, 1, 1, 2, 0x2222, true, false, 5},
        // Same material as the landscape: the snap arm returns without drawing.
        {"slide_same_mat", 1, 1, 8, 9, 0, 1, 1, 2, 0x2222, true, false, 5},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                insert_check::g_grid[gy][gx] = 0;
        if (c.floor)
            for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                if (gx != c.hole) insert_check::g_grid[10][gx] = 3;
        if (c.walled)
            for (int32_t gy = 0; gy < insert_check::GridHgt; gy++)
                for (int32_t gx = 0; gx < insert_check::GridWdt; gx++)
                    if (gx != 8) insert_check::g_grid[gy][gx] = 3;
        insert_check::g_smoke = 0;

        FixedRandom(c.seed);
        Randomize3();
        const int32_t rnd3_before = RandomCount;

        int32_t iX = c.x, iY = c.y;
        C4Fixed xdir = itofix(c.xdir_n, c.xdir_p);
        C4Fixed ydir = itofix(c.ydir_n, c.ydir_p);
        int32_t pxs_mat = c.pxs_mat;
        bool pos_changed = false;
        const bool verdict = insert_check::mrfInsertCheck(iX, iY, xdir, ydir, pxs_mat,
                                                          c.ls_mat, &pos_changed);

        printf("{\"name\":\"%s\",\"pxs_mat\":%d,\"ls_mat\":%d,\"x0\":%d,\"y0\":%d,"
               "\"xdir0\":%d,\"ydir0\":%d,\"seed\":%d,\"floor\":%s,\"walled\":%s,"
               "\"verdict\":%s,\"x\":%d,\"y\":%d,\"xdir\":%d,\"ydir\":%d,"
               "\"pos_changed\":%s,\"smoke\":%d,\"draws\":%d,\"hole\":%d}",
               c.name, c.pxs_mat, c.ls_mat, c.x, c.y,
               itofix(c.xdir_n, c.xdir_p).val, itofix(c.ydir_n, c.ydir_p).val, c.seed,
               c.floor ? "true" : "false", c.walled ? "true" : "false",
               verdict ? "true" : "false", iX, iY, xdir.val, ydir.val,
               pos_changed ? "true" : "false", insert_check::g_smoke,
               RandomCount - rnd3_before, c.hole);
    }
    printf("]");
}


// Steps one pixel for a fixed number of frames, recording the raw fixed state
// after every frame together with the RNG ledger, so a wrong draw count shows
// up even when the position happens to agree.
static void printPxsExecuteCases()
{
    printf("\"pxs_execute\":[");
    struct Case
    {
        const char *name;
        int32_t mat;
        int32_t x_n, x_p, y_n, y_p;
        int32_t xdir_n, xdir_p, ydir_n, ydir_p;
        int32_t wind;
        int32_t seed;
        int32_t frames;
        bool floor;
    };
    const Case cases[] = {
        // Free fall in still air: gravity only, two draws per frame.
        {"fall_still_air", 2, 4, 1, 1, 1, 0, 1, 0, 1, 0, 0x1234, 6, false},
        // Invalid material deactivates before anything else runs.
        {"invalid_material", 7, 4, 1, 1, 1, 0, 1, 0, 1, 0, 0x1234, 1, false},
        // Out of bounds below the map deactivates on the bounds check.
        {"out_of_bounds", 2, 4, 1, 40, 1, 0, 1, 0, 1, 0, 0x1234, 1, false},
    };
    bool first = true;
    for (const auto &c : cases)
    {
        if (!first) printf(",");
        first = false;

        for (int32_t gy = 0; gy < pxs_exec::GridHgt; gy++)
            for (int32_t gx = 0; gx < pxs_exec::GridWdt; gx++)
                pxs_exec::g_grid[gy][gx] = 0;
        for (int32_t gx = 0; gx < pxs_exec::GridWdt; gx++)
            for (int32_t gy = 0; gy < pxs_exec::GridHgt; gy++)
                pxs_exec::g_pix_cnt[gx][gy] = 0;
        if (c.floor)
            for (int32_t gx = 0; gx < pxs_exec::GridWdt; gx++)
            {
                pxs_exec::g_grid[10][gx] = 1;
                pxs_exec::g_pix_cnt[gx][10] = 1;
            }
        pxs_exec::g_wind = c.wind;

        FixedRandom(c.seed);
        pxs_exec::C4PXS pix;
        pix.Mat = c.mat;
        pix.x = itofix(c.x_n, c.x_p);
        pix.y = itofix(c.y_n, c.y_p);
        pix.xdir = itofix(c.xdir_n, c.xdir_p);
        pix.ydir = itofix(c.ydir_n, c.ydir_p);

        printf("{\"name\":\"%s\",\"mat\":%d,\"wind\":%d,\"seed\":%d,"
               "\"x0\":%d,\"y0\":%d,\"xdir0\":%d,\"ydir0\":%d,\"frames\":[",
               c.name, c.mat, c.wind, c.seed,
               pix.x.val, pix.y.val, pix.xdir.val, pix.ydir.val);
        for (int32_t f = 0; f < c.frames; f++)
        {
            pix.Execute();
            if (f) printf(",");
            printf("{\"x\":%d,\"y\":%d,\"xdir\":%d,\"ydir\":%d,\"mat\":%d,"
                   "\"deactivated\":%s,\"random_count\":%d}",
                   pix.x.val, pix.y.val, pix.xdir.val, pix.ydir.val, pix.Mat,
                   pix.deactivated ? "true" : "false", RandomCount);
        }
        printf("]}");
    }
    printf("]");
}






int main()
{
    printf("{\n");

    // 1. itofix: whole-integer and precision-denominated construction.
    //    Covers gravity/velocity precision (default 10, FIXED100, FIXED256).
    // C4PXSSystem::New's slot choice, including reuse of freed slots
    // (C4PXS.cpp:181-204, 426-437). The sequence deliberately frees out of
    // order so a naive bump allocator disagrees on the very next call.
    arr_begin("pxs_allocation");
    {
        using namespace pxs_allocation;
        C4PXSSystem system;
        std::vector<C4PXS *> live;
        const auto take = [&](const char *step)
        {
            C4PXS *pxs = system.New();
            int chunk = -1, slot = -1;
            if (pxs)
            {
                pxs->Mat = 1;
                system.locate(pxs, chunk, slot);
                live.push_back(pxs);
            }
            sep();
            printf("{\"step\":\"%s\",\"chunk\":%d,\"slot\":%d}", step, chunk, slot);
        };
        const auto drop = [&](size_t index, const char *step)
        {
            int chunk = -1, slot = -1;
            if (index < live.size())
            {
                C4PXS *pxs = live[index];
                system.locate(pxs, chunk, slot);
                pxs->Mat = MNone;
                system.Delete(pxs);
            }
            sep();
            printf("{\"step\":\"%s\",\"chunk\":%d,\"slot\":%d}", step, chunk, slot);
        };

        take("new0");
        take("new1");
        take("new2");
        take("new3");
        // Free the middle two, high index first, so reuse order is visibly the
        // lowest free slot rather than the most recently freed one.
        drop(2, "free2");
        drop(1, "free1");
        take("reuse_a");
        take("reuse_b");
        take("append");
    }
    arr_end();
    printf(",\n");

    // mrfPoof's synchronised draws (C4Material.cpp:663-688). Each case runs
    // the real arm from a known RNG seed and records what it consumed.
    arr_begin("material_poof_reaction");
    {
        const int seeds[] = {0, 1, 2, 3, 7, 11, 42, 1234};
        // `meePXSMove` is excluded: it runs the insert check, which walks the
        // landscape through FindMatSlide.
        const poof_reaction::MaterialInteractionEvent events[] = {
            poof_reaction::meeMassMove,
            poof_reaction::meePXSPos,
        };
        poof_reaction::C4MaterialMap map;
        for (int seed : seeds)
            for (poof_reaction::MaterialInteractionEvent event : events)
            {
                FixedRandom(seed);
                // Rnd3 reads a table Randomize3 fills; without this it is all
                // zeros and every draw answers 0, which would pin an artefact
                // of the harness rather than the reaction.
                Randomize3();
                poof_reaction::g_extractions = 0;
                poof_reaction::g_smoke = 0;
                poof_reaction::g_sound = 0;
                poof_reaction::C4MaterialReaction reaction{false};
                int32_t x = 30, y = 40, pxs_mat = 1;
                C4Fixed xdir = itofix(0), ydir = itofix(0);
                bool pos_changed = false;
                const bool handled = map.mrfPoof(&reaction, x, y, 11, 12, xdir, ydir, pxs_mat, 2,
                                                 event, &pos_changed);
                sep();
                printf("{\"seed\":%d,\"event\":%d,\"handled\":%d,\"extractions\":%d,"
                       "\"smoke\":%d,\"sound\":%d,\"random_count\":%d,\"random_hold\":%u}",
                       seed, static_cast<int>(event), handled ? 1 : 0,
                       poof_reaction::g_extractions, poof_reaction::g_smoke,
                       poof_reaction::g_sound, RandomCount, static_cast<unsigned>(RandomHold));
            }
    }
    arr_end();
    printf(",\n");

    // C4MassMoverSet::Create's cyclic slot scan (C4MassMover.cpp:67-94). The
    // search starts *after* CreatePtr, so a slot freed behind the cursor is not
    // reused until the pointer comes round to it — the opposite of the PXS
    // allocator, which always takes the lowest free slot. A port that made the
    // two consistent would pass one section and fail the other.
    arr_begin("mass_mover_allocation");
    {
        static mover_allocation::C4MassMoverSet set;
        const auto take = [&](const char *step)
        {
            const bool ok = set.Create(7, 9);
            sep();
            printf("{\"step\":\"%s\",\"ok\":%d,\"create_ptr\":%d}", step, ok ? 1 : 0,
                   set.CreatePtr);
        };
        const auto free_slot = [&](int32_t slot, const char *step)
        {
            set.Set[slot].Mat = mover_allocation::MNone;
            sep();
            printf("{\"step\":\"%s\",\"ok\":1,\"create_ptr\":%d}", step, set.CreatePtr);
        };

        take("first");
        take("second");
        take("third");
        // Freeing behind the cursor does not bring the cursor back: the next
        // scan starts at CreatePtr + 1 and takes the next slot forward.
        free_slot(1, "free_behind");
        take("takes_next_forward");
        free_slot(2, "free_behind_again");
        take("still_forward");

        // Fill the rest of the chunk so the only free slots are the two behind
        // the cursor. Now the scan has to wrap the chunk end to find them, and
        // it takes the lower one first because it reaches it first going
        // forward from the wrap.
        while (set.Create(7, 9))
        {
        }
        sep();
        printf("{\"step\":\"full\",\"ok\":0,\"create_ptr\":%d}", set.CreatePtr);
        free_slot(1, "free_for_wrap");
        take("wraps_to_freed");
    }
    arr_end();
    printf(",\n");

    // Splash's draw stream (C4Effect.cpp:801-836). Each case records every
    // bubble and cast in order plus the synchronised ledger, because the number
    // of draws is not a function of `amt` alone: the first iteration's
    // extraction empties the pixel the liquid test reads, so later iterations
    // take two draws instead of four.
    arr_begin("splash_effect");
    {
        struct Case
        {
            const char *name;
            int32_t seed;
            int32_t water_top;
            int32_t floor_top;
            int32_t liquid_mat;
            int32_t tx, ty;
            int32_t amt;
        };
        const Case cases[] = {
            // Deep water, free sky above: the full four-draw first iteration
            // followed by two-draw iterations once the pixel is gone.
            {"deep_water", 1, 18, splash_effect::GridHgt, 0, 4, 20, 5},
            // One bubble only — the boundary where the draw count drops.
            {"single_bubble", 1, 18, splash_effect::GridHgt, 0, 4, 20, 1},
            // Loud splash: the >= 20 sound branch, and enough iterations to
            // show the stream settling at two draws each.
            {"loud", 7, 18, splash_effect::GridHgt, 0, 4, 20, 20},
            // Nothing to splash into: amt of zero still probes the landscape
            // but must draw nothing.
            {"no_amount", 7, 18, splash_effect::GridHgt, 0, 4, 20, 0},
            // Roofed over: GBackSemiSolid(tx, ty - 15) returns before any draw.
            {"roofed", 3, 4, splash_effect::GridHgt, 0, 4, 20, 5},
            // Liquid but not instable: the loop is skipped entirely, yet the
            // sound still plays. No draws either way.
            {"not_instable", 3, 18, splash_effect::GridHgt, 1, 4, 20, 5},
            // Above the water line: GBackMat is sky, so MatValid fails.
            {"in_sky", 3, 30, splash_effect::GridHgt, 0, 4, 20, 5},
            // Shallow pool over granite: the surface scan stops at the water
            // top rather than running the full 20 rows.
            {"shallow", 11, 19, 22, 0, 4, 20, 4},
        };

        for (const Case &c : cases)
        {
            FixedRandom(c.seed);
            splash_effect::reset_grid(c.water_top, c.floor_top, c.liquid_mat);
            splash_effect::Splash(c.tx, c.ty, c.amt, nullptr);

            sep();
            printf("{\"case\":\"%s\",\"seed\":%d,\"amt\":%d,\"bubbles\":[", c.name, c.seed,
                   c.amt);
            for (int32_t i = 0; i < splash_effect::g_bubble_count; ++i)
                printf("%s[%d,%d]", i ? "," : "", splash_effect::g_bubbles[i].x,
                       splash_effect::g_bubbles[i].y);
            printf("],\"casts\":[");
            for (int32_t i = 0; i < splash_effect::g_cast_count; ++i)
                printf("%s[%d,%d,%d,%d,%d]", i ? "," : "", splash_effect::g_casts[i].mat,
                       splash_effect::g_casts[i].x, splash_effect::g_casts[i].y,
                       splash_effect::g_casts[i].xdir, splash_effect::g_casts[i].ydir);
            printf("],\"extractions\":%d,\"sound\":\"%s\",\"random_count\":%d,"
                   "\"random_hold\":%u}",
                   splash_effect::g_extractions, splash_effect::g_sound, RandomCount,
                   static_cast<unsigned>(RandomHold));
        }
    }
    arr_end();
    printf(",\n");

    // C4Object::UpdateInLiquid (C4Object.cpp:6093-6110) and the probe it reads
    // through (:5632-5635). Entry is edge-triggered and carries the splash;
    // leaving is a bare flag clear. The probe is `y + Def->Float * Con /
    // FullCon - 1`, so construction and Float move the moment an object counts
    // as swimming.
    arr_begin("in_liquid_transition");
    {
        struct Case
        {
            const char *name;
            int32_t seed;
            int32_t water_top;
            int32_t y;
            int32_t in_liquid;
            int32_t con;
            int32_t float_line;
            int32_t mass;
            uint32_t ocf;
            int32_t wdt, hgt;
        };
        const uint32_t hit = splash_effect::OCF_HitSpeed2;
        const int32_t full = splash_effect::FullCon;
        const Case cases[] = {
            // Enters: fast and heavy enough, so the splash fires.
            {"enter_splash", 1, 18, 20, 0, full, 0, 10, hit, 8, 10},
            // Enters without the hit-speed flag: no splash, no draws.
            {"enter_no_hitspeed", 1, 18, 20, 0, full, 0, 10, 0, 8, 10},
            // Enters at the mass boundary: `Mass > 3` excludes exactly 3.
            {"enter_mass_boundary", 1, 18, 20, 0, full, 0, 3, hit, 8, 10},
            {"enter_mass_above", 1, 18, 20, 0, full, 0, 4, hit, 8, 10},
            // Already wet: entry is edge-triggered, so nothing happens.
            {"stays_wet", 1, 18, 20, 1, full, 0, 10, hit, 8, 10},
            // Dry and stays dry.
            {"stays_dry", 1, 30, 20, 0, full, 0, 10, hit, 8, 10},
            // Leaves: the flag clears and nothing else runs.
            {"leaves", 1, 30, 20, 1, full, 0, 10, hit, 8, 10},
            // Float lifts the probe INTO the water for an object whose own y is
            // still above it.
            {"float_reaches_water", 1, 18, 14, 0, full, 6, 10, hit, 8, 10},
            // Half-built, so Float * Con / FullCon halves and the same object
            // no longer reaches it.
            {"half_con_falls_short", 1, 18, 14, 0, full / 2, 6, 10, hit, 8, 10},
            // The splash amount is min(Wdt * Hgt / 10, 20): a large object
            // clamps, and the clamp is what decides the draw count.
            {"large_object_clamps", 5, 18, 20, 0, full, 0, 10, hit, 40, 40},
            // A small one takes the unclamped amount.
            {"small_object_amount", 5, 18, 20, 0, full, 0, 10, hit, 5, 6},
        };

        for (const Case &c : cases)
        {
            FixedRandom(c.seed);
            splash_effect::reset_grid(c.water_top, splash_effect::GridHgt, 0);

            splash_effect::DefStub def;
            def.Float = c.float_line;
            splash_effect::C4Object obj;
            obj.x = 4;
            obj.y = c.y;
            obj.Con = c.con;
            obj.Mass = c.mass;
            obj.InLiquid = c.in_liquid;
            obj.OCF = c.ocf;
            obj.Shape.Wdt = c.wdt;
            obj.Shape.Hgt = c.hgt;
            obj.Def = &def;

            const int32_t probe_y = obj.y + def.Float * obj.Con / splash_effect::FullCon - 1;
            const bool wet = obj.IsInLiquidCheck();
            obj.UpdateInLiquid();

            sep();
            printf("{\"case\":\"%s\",\"seed\":%d,\"probe_y\":%d,\"wet\":%d,"
                   "\"in_liquid_before\":%d,\"in_liquid\":%d,\"bubbles\":%d,\"casts\":%d,"
                   "\"random_count\":%d,\"random_hold\":%u}",
                   c.name, c.seed, probe_y, wet ? 1 : 0, c.in_liquid, obj.InLiquid,
                   splash_effect::g_bubble_count, splash_effect::g_cast_count, RandomCount,
                   static_cast<unsigned>(RandomHold));
        }
    }
    arr_end();
    printf(",\n");

    // C4Weather::Execute's disaster block (C4Weather.cpp:104-148). Each case
    // runs twenty Tick10 ticks on one seeded stream and records the events and
    // the ledger after every tick, so a port that reordered the four gates, or
    // skipped a level test at level zero, diverges on the very first tick that
    // differs.
    arr_begin("weather_execute");
    {
        struct Case
        {
            const char *name;
            int32_t seed;
            int32_t meteorite;
            int32_t lightning;
            int32_t earthquake;
            int32_t volcano;
            int32_t top_open;
        };
        const Case cases[] = {
            // Every level zero: no disaster can fire, yet each gate that opens
            // still spends its second draw on the level test.
            {"all_levels_zero", 3, 0, 0, 0, 0, 1},
            // Everything certain once the outer gate opens.
            {"all_levels_full", 3, 100, 100, 100, 100, 1},
            // A cave landscape moves the meteor's spawn and gives it a
            // downward ydir (C4Weather.cpp:117-119).
            {"all_levels_full_cave", 3, 100, 100, 100, 100, 0},
            // Mixed levels, including one zero, so the tick's draw count and
            // its event list come apart.
            {"mixed_levels", 11, 50, 100, 0, 25, 1},
        };

        for (const Case &c : cases)
        {
            FixedRandom(c.seed);
            // The engine fills the Rnd3 table at startup, which spends 500
            // draws. Execute never reads that table, but the port's
            // Engine::with_seed fills it too, so the ledgers only line up if
            // the oracle does the same.
            Randomize3();
            weather_execute::Tick10 = 0;
            weather_execute::Tick35 = 1;
            weather_execute::Tick1000 = 1;
            weather_execute::Game.Landscape.TopOpen = c.top_open;

            weather_execute::C4Weather weather;
            weather.MeteoriteLevel = c.meteorite;
            weather.LightningLevel = c.lightning;
            weather.EarthquakeLevel = c.earthquake;
            weather.VolcanoLevel = c.volcano;

            sep();
            printf("{\"case\":\"%s\",\"seed\":%d,\"width\":%d,\"height\":%d,"
                   "\"top_open\":%d,\"meteorite\":%d,\"lightning\":%d,\"earthquake\":%d,"
                   "\"volcano\":%d,\"ticks\":[",
                   c.name, c.seed, weather_execute::GBackWdt, weather_execute::GBackHgt,
                   c.top_open, c.meteorite, c.lightning, c.earthquake, c.volcano);
            // Four hundred ticks: the outer gates are one in thirty-five to one
            // in sixty, so a short run would pin nothing but "four draws a
            // tick". Only the ticks that produced an event are recorded, plus
            // the ledger sampled every fortieth tick and at the end — enough to
            // catch a reordered gate or a missing level test without carrying
            // four hundred rows per case.
            bool first_tick = true;
            for (int32_t tick = 0; tick < 400; ++tick)
            {
                weather_execute::g_event_count = 0;
                weather.Execute();
                const bool sampled = (tick % 40 == 39) || tick == 399;
                if (!weather_execute::g_event_count && !sampled) continue;
                if (!first_tick) printf(",");
                first_tick = false;
                printf("{\"tick\":%d,\"random_count\":%d,\"random_hold\":%u,\"events\":[",
                       tick, RandomCount, static_cast<unsigned>(RandomHold));
                for (int32_t e = 0; e < weather_execute::g_event_count; ++e)
                    printf("%s{\"kind\":\"%s\",\"a\":%d,\"b\":%d,\"c\":%d,\"d\":%d}",
                           e ? "," : "", weather_execute::g_events[e].kind,
                           weather_execute::g_events[e].a, weather_execute::g_events[e].b,
                           weather_execute::g_events[e].c, weather_execute::g_events[e].d);
                printf("]}");
            }
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    // C4Shape::ContactCheck (C4Shape.cpp:370-406) over a 24x16 material grid.
    // The landscape is earth from y=10 down with a water pocket and a granite
    // pillar, and every case probes one shape at one position: the vertex loop
    // order, the four neighbour probes per contacting vertex, the
    // CNAT_NoCollision skip, and the closed-border MCVehic answer.
    arr_begin("shape_contact_check");
    {
        using namespace shape_contact;

        struct Vertex
        {
            int32_t x, y, cnat;
        };
        struct Case
        {
            const char *name;
            int32_t at_x, at_y;
            int32_t contact_density;
            int32_t left_open, right_open;
            bool top_open, bottom_open;
            int32_t vtx_num;
            Vertex vertices[MaxVertex];
        };

        const int32_t Left = CNAT_Left, Right = CNAT_Right, Top = CNAT_Top, Bottom = CNAT_Bottom;
        const int32_t NoCollision = CNAT_NoCollision;
        const Case cases[] = {
            // A single bottom vertex resting on the earth surface: centre plus
            // the bottom neighbour, and nothing else.
            {"on_surface", 8, 10, SolidDensity, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // One pixel higher it is in sky, so no contact at all.
            {"above_surface", 8, 9, SolidDensity, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Buried: every neighbour answers solid too.
            {"buried", 8, 12, SolidDensity, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Water is density 30. At the solid threshold it is not contact; at
            // a liquid threshold the SAME pixel is.
            {"water_solid_threshold", 4, 12, SolidDensity, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            {"water_liquid_threshold", 4, 12, 25, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Four vertices around a Clonk-like shape standing on the surface:
            // ContactCNAT is the OR of the CONTACTING vertices' own CNATs, and
            // ContactCount counts them.
            {"standing_shape", 8, 8, SolidDensity, 0, 0, true, false, 4,
             {{-3, 2, Left}, {3, 2, Right}, {0, -3, Top}, {0, 2, Bottom}}},
            // The same shape pushed into the granite pillar so a side vertex
            // contacts as well.
            {"against_pillar", 15, 8, SolidDensity, 0, 0, true, false, 4,
             {{-3, 2, Left}, {3, 2, Right}, {0, -3, Top}, {0, 2, Bottom}}},
            // A CNAT_NoCollision vertex sitting in solid ground is skipped
            // entirely — it neither contacts nor gets a material recorded.
            {"no_collision_vertex", 8, 12, SolidDensity, 0, 0, true, false, 2,
             {{0, 0, Bottom | NoCollision}, {2, -4, Bottom}}},
            // Off the left edge with the border CLOSED: the border answers
            // MCVehic, so the vertex contacts empty space.
            {"closed_left_border", 0, 4, SolidDensity, 0, 0, true, false, 1, {{-1, 0, Left}}},
            // The same position with the left border open to y=8 is sky.
            {"open_left_border", 0, 4, SolidDensity, 8, 0, true, false, 1, {{-1, 0, Left}}},
            // Above the map with TopOpen is sky; with it closed the ceiling is
            // solid.
            {"open_top_border", 8, 0, SolidDensity, 0, 0, true, false, 1, {{0, -1, Top}}},
            {"closed_top_border", 8, 0, SolidDensity, 0, 0, false, false, 1, {{0, -1, Top}}},
            // Below the map: BottomOpen is false in these fixtures, so the
            // floor of the world is solid.
            {"closed_bottom_border", 8, 15, SolidDensity, 0, 0, true, false, 1, {{0, 1, Bottom}}},
        };

        for (const Case &c : cases)
        {
            // Sky above y=10, earth below, a water pocket at x=3..5 and a
            // granite pillar at x=17..18.
            for (int32_t y = 0; y < GridHgt; ++y)
                for (int32_t x = 0; x < GridWdt; ++x)
                {
                    int32_t mat = y >= 10 ? 1 : 0;
                    if (y >= 11 && x >= 3 && x <= 5) mat = 2;
                    if (x >= 17 && x <= 18 && y >= 6) mat = 1;
                    g_grid[y][x] = mat;
                }

            g_left_open = c.left_open;
            g_right_open = c.right_open;
            g_top_open = c.top_open;
            g_bottom_open = c.bottom_open;

            C4Shape shape;
            shape.VtxNum = c.vtx_num;
            shape.ContactDensity = c.contact_density;
            for (int32_t v = 0; v < c.vtx_num; ++v)
            {
                shape.VtxX[v] = c.vertices[v].x;
                shape.VtxY[v] = c.vertices[v].y;
                shape.VtxCNAT[v] = c.vertices[v].cnat;
                shape.VtxContactMat[v] = MNone;
            }

            const bool any = shape.ContactCheck(c.at_x, c.at_y);

            sep();
            printf("{\"case\":\"%s\",\"at_x\":%d,\"at_y\":%d,\"contact_density\":%d,"
                   "\"left_open\":%d,\"right_open\":%d,\"top_open\":%d,\"bottom_open\":%d,"
                   "\"any\":%d,\"contact_cnat\":%d,\"contact_count\":%d,\"vertices\":[",
                   c.name, c.at_x, c.at_y, c.contact_density, c.left_open, c.right_open,
                   c.top_open ? 1 : 0, c.bottom_open ? 1 : 0, any ? 1 : 0, shape.ContactCNAT,
                   shape.ContactCount);
            for (int32_t v = 0; v < c.vtx_num; ++v)
                printf("%s{\"x\":%d,\"y\":%d,\"cnat\":%d,\"contact_cnat\":%d,\"mat\":%d}",
                       v ? "," : "", c.vertices[v].x, c.vertices[v].y, c.vertices[v].cnat,
                       shape.VtxContactCNAT[v], shape.VtxContactMat[v]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    // C4Object::TargetBounds (C4Movement.cpp:128-164). The bound that fires
    // decides which velocity component is zeroed — CNAT_Left/CNAT_Right clear
    // xdir, anything else clears ydir — and fires a Contact call. The crossed
    // case pins that both bounds fire, low first.
    // C4Shape::Attach (C4Shape.cpp:165-271), the search attached movement runs
    // instead of the ordinary collision loop. The two branches differ in a way
    // that matters: the old-style search loops vertices OUTSIDE and the range
    // inside, so a second matching vertex starts from the position the first
    // one already moved to, while CNAT_MultiAttach loops the range outside and
    // takes the nearest attachment across all vertices, breaking both loops.
    arr_begin("shape_attach");
    {
        using namespace shape_contact;

        struct Vertex
        {
            int32_t x, y, cnat;
        };
        struct Case
        {
            const char *name;
            int32_t at_x, at_y;
            int32_t attach;
            int32_t left_open, right_open;
            bool top_open, bottom_open;
            int32_t vtx_num;
            Vertex vertices[MaxVertex];
        };

        const int32_t Left = CNAT_Left, Right = CNAT_Right, Top = CNAT_Top, Bottom = CNAT_Bottom;
        const int32_t Multi = CNAT_MultiAttach;
        const Case cases[] = {
            // Standing three pixels above the earth surface: the downward scan
            // starts five above and walks down, so it lands on the surface and
            // corrects cy.
            {"bottom_from_above", 8, 7, Bottom, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Already resting on it: the scan still runs and still reports the
            // pixel it attached to.
            {"bottom_on_surface", 8, 9, Bottom, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Too far above for the range to reach.
            {"bottom_out_of_range", 8, 2, Bottom, 0, 0, true, false, 1, {{0, 0, Bottom}}},
            // Sideways onto the pillar at x=17..18.
            {"right_onto_pillar", 14, 8, Right, 0, 0, true, false, 1, {{0, 0, Right}}},
            {"left_onto_pillar", 21, 8, Left, 0, 0, true, false, 1, {{0, 0, Left}}},
            // Upward against the pillar's underside.
            {"top_under_pillar", 17, 10, Top, 0, 0, true, false, 1, {{0, 0, Top}}},
            // A vertex whose CNAT does not match the requested direction is
            // never considered.
            {"cnat_mismatch", 8, 7, Bottom, 0, 0, true, false, 1, {{0, 0, Top}}},
            // Two matching vertices, old style: the loop moves the position for
            // the first, then runs the second from THERE.
            {"two_vertices_old_style", 8, 7, Bottom, 0, 0, true, false, 2,
             {{-4, 0, Bottom}, {4, -2, Bottom}}},
            // The same shape and position with CNAT_MultiAttach: the range is
            // the outer loop, so the nearest match across both vertices wins.
            {"two_vertices_multi", 8, 7, Bottom | Multi, 0, 0, true, false, 2,
             {{-4, 0, Bottom}, {4, -2, Bottom}}},
            // A closed left border answers solid to a density probe, but Attach
            // additionally requires `ax >= 0`, so an object can CONTACT the
            // edge of the map without attaching to it.
            {"closed_border_no_attach", 0, 4, Left, 0, 0, true, false, 1, {{0, 0, Left}}},
            // No direction bits at all: the switch leaves xcd and ycd zero, so
            // the range is empty and nothing is searched.
            {"no_direction", 8, 7, 0, 0, 0, true, false, 1, {{0, 0, Bottom}}},
        };

        for (const Case &c : cases)
        {
            for (int32_t y = 0; y < GridHgt; ++y)
                for (int32_t x = 0; x < GridWdt; ++x)
                {
                    int32_t byte = y >= 10 ? 1 : 0;
                    if (y >= 11 && x >= 3 && x <= 5) byte = 2;
                    if (x >= 17 && x <= 18 && y >= 6) byte = 1;
                    g_grid[y][x] = byte;
                }

            g_left_open = c.left_open;
            g_right_open = c.right_open;
            g_top_open = c.top_open;
            g_bottom_open = c.bottom_open;

            C4Shape shape;
            shape.VtxNum = c.vtx_num;
            shape.ContactDensity = SolidDensity;
            for (int32_t v = 0; v < c.vtx_num; ++v)
            {
                shape.VtxX[v] = c.vertices[v].x;
                shape.VtxY[v] = c.vertices[v].y;
                shape.VtxCNAT[v] = c.vertices[v].cnat;
            }

            int32_t cx = c.at_x, cy = c.at_y;
            const bool attached = shape.Attach(cx, cy, static_cast<uint8_t>(c.attach));

            sep();
            printf("{\"case\":\"%s\",\"at_x\":%d,\"at_y\":%d,\"attach\":%d,"
                   "\"left_open\":%d,\"right_open\":%d,\"top_open\":%d,\"bottom_open\":%d,"
                   "\"attached\":%d,\"x\":%d,\"y\":%d,\"attach_mat\":%d,\"attach_x\":%d,"
                   "\"attach_y\":%d,\"attach_vtx\":%d,\"vertices\":[",
                   c.name, c.at_x, c.at_y, c.attach, c.left_open, c.right_open,
                   c.top_open ? 1 : 0, c.bottom_open ? 1 : 0, attached ? 1 : 0, cx, cy,
                   shape.AttachMat, shape.iAttachX, shape.iAttachY, shape.iAttachVtx);
            for (int32_t v = 0; v < c.vtx_num; ++v)
                printf("%s{\"x\":%d,\"y\":%d,\"cnat\":%d}", v ? "," : "", c.vertices[v].x,
                       c.vertices[v].y, c.vertices[v].cnat);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    // C4Object::Enter, Exit and Collect (C4Object.cpp:1532-1563, 1566-1637,
    // 5693-5717). Each case records the exact sequence of script calls and
    // bookkeeping the lifted bodies performed, so a reordered mutation or a
    // missing post-callback `Status` re-check shows up as a different list
    // rather than as a subtly different end state.
    // C4Effect::Check (C4Effect.cpp:271-316). Three effects sit in the list at
    // different priorities; each case configures what their checker callbacks
    // answer and records the exact sequence of calls the negotiation made,
    // together with the number it returned.
    // C4Effect::Execute (C4Effect.cpp:319-363), the per-frame pass. It walks
    // the list unlinking dead effects as it goes, advances each survivor's
    // clock FIRST, then fires the timer only when the new time lands exactly on
    // an interval boundary — and kills outright any effect that has an interval
    // but no timer function at all.
    // C4Object::AssignRemoval (C4Object.cpp:240-320), the object teardown. The
    // order is the parity fact: the CONTAINER's ContentsDestruction runs before
    // the object's own Destruction, effects clear next, and each of those steps
    // is followed by a Status re-check because the callback may already have
    // deleted the object. Contents are torn down BEFORE the object leaves its
    // own container.
    // C4Object::AssignDeath (C4Object.cpp:1164-1205). Two orderings carry it:
    // the death-causing player is read BEFORE the effect clear, because the
    // effects can meddle with the flags, and it is handed to the Death callback
    // at the very END; and `Alive` is cleared BEFORE that clear so a dying
    // object cannot recurse into its own death. An effect that puts the object
    // back on its feet aborts the kill unless it was forced.
    // C4Object::ChangeDef (C4Object.cpp:1207-1255), compiled beside the real
    // Enter/Exit. The headline is the container round-trip: the object leaves
    // and re-enters with fCalls=false, so a definition change inside a
    // container fires NEITHER Ejection/Departure on the way out NOR
    // Collection2/Entrance on the way back — a script watching its contents
    // sees nothing at all.
    // C4MouseControl::UpdateCursorTarget's OCF priority cascade
    // (C4MouseControl.cpp:481-521). Every rule is an unconditional overwrite,
    // so the LAST match wins: a candidate carrying several OCF bits ends on the
    // rule furthest down the ladder, and adding a bit can only move the cursor
    // later in that order, never earlier.
    arr_begin("mouse_cursor_cascade");
    {
        using namespace mouse_cursor;

        struct Case
        {
            const char *name;
            uint32_t ocf;          // the mask UpdateCursorTarget assembled
            uint32_t target_ocf;   // the candidate's own OCF
            int32_t category;
            int32_t owner;
            bool alive;
            bool hostile;
            bool in_crew;
            bool pushing_target;   // the player's cursor is pushing this object
            bool has_player_cursor;
            int32_t player;
            int32_t x, y;          // pointer position
        };

        const int32_t ObjX = 100, ObjY = 100;
        const Case cases[] = {
            // Nothing matches: the default object cursor stands.
            {"no_match_keeps_crosshair", 0, 0, 0, -1, false, false, false, false, false, -1,
             ObjX, ObjY},
            // A container the candidate can be entered through.
            {"container_with_entrance", OCF_Container, OCF_Entrance, 0, -1, false, false, false,
             false, false, -1, ObjX, ObjY},
            // The container rule needs the candidate's OWN entrance bit.
            {"container_without_entrance", OCF_Container, 0, 0, -1, false, false, false, false,
             false, -1, ObjX, ObjY},
            // Grab, and its Ungrab form when the player's cursor is already
            // pushing this very object.
            {"grab", OCF_Grab, 0, 0, -1, false, false, false, false, true, 0, ObjX, ObjY},
            {"ungrab_when_pushing_it", OCF_Grab, 0, 0, -1, false, false, false, true, true, 0,
             ObjX, ObjY},
            // Carryable overwrites grab, and the in-solid form overwrites that.
            {"carryable_beats_grab", OCF_Grab | OCF_Carryable, 0, 0, -1, false, false, false,
             false, false, -1, ObjX, ObjY},
            {"dig_object_when_in_solid", OCF_Carryable | OCF_InSolid, 0, 0, -1, false, false,
             false, false, false, -1, ObjX, ObjY},
            // Chop has a REDUCED range: a third of the shape's width either
            // side, and vertically from half the width above to a third below.
            {"chop_inside_range", OCF_Carryable | OCF_Chop, 0, 0, -1, false, false, false, false,
             false, -1, ObjX, ObjY},
            {"chop_outside_range", OCF_Carryable | OCF_Chop, 0, 0, -1, false, false, false,
             false, false, -1, ObjX + 15, ObjY},
            // The second Entrance rule reads the ASSEMBLED mask, not the
            // candidate's own bit, and sits below chop.
            {"entrance_beats_chop", OCF_Chop | OCF_Entrance, 0, 0, -1, false, false, false, false,
             false, -1, ObjX, ObjY},
            // Build overwrites everything above it.
            {"construct_beats_entrance", OCF_Entrance | OCF_Construct, 0, 0, -1, false, false,
             false, false, false, -1, ObjX, ObjY},
            // Select for a crew member of this player...
            {"crew_member_selects", OCF_Construct | OCF_Alive, 0, 0, 0, true, false, true, false,
             false, 0, ObjX, ObjY},
            // ...and for anything carrying the MouseSelect category, with no
            // player needed at all.
            {"mouse_select_category", OCF_Construct, 0, C4D_MouseSelect, -1, false, false, false,
             false, false, -1, ObjX, ObjY},
            // Attack is last, so a hostile living candidate outranks even a
            // crew Select — and it needs the candidate to be actually alive.
            {"hostile_alive_attacks", OCF_Alive, 0, C4D_MouseSelect, 1, true, true, true, false,
             false, 0, ObjX, ObjY},
            {"hostile_dead_does_not_attack", OCF_Alive, 0, C4D_MouseSelect, 1, false, true, true,
             false, false, 0, ObjX, ObjY},
            {"friendly_alive_does_not_attack", OCF_Alive, 0, C4D_MouseSelect, 0, true, false,
             true, false, false, 0, ObjX, ObjY},
        };

        for (const Case &c : cases)
        {
            mouse_cursor::C4Object target;
            target.OCF = c.target_ocf;
            target.Category = c.category;
            target.Owner = c.owner;
            target.Alive = c.alive;

            mouse_cursor::C4Object cursor_object;
            cursor_object.Procedure = DFA_PUSH;
            cursor_object.Action.Target = c.pushing_target ? &target : nullptr;

            mouse_cursor::PlayerStub player;
            player.CrewMember = c.in_crew ? &target : nullptr;
            Game.Players.Held = &player;
            g_hostile = c.hostile;

            const int32_t result = run(
                c.ocf, &target, c.has_player_cursor || c.pushing_target ? &cursor_object : nullptr,
                c.player, c.x, c.y, ObjX, ObjY);

            sep();
            printf("{\"case\":\"%s\",\"ocf\":%u,\"target_ocf\":%u,\"category\":%d,"
                   "\"owner\":%d,\"alive\":%d,\"hostile\":%d,\"in_crew\":%d,"
                   "\"pushing\":%d,\"player\":%d,\"dx\":%d,\"cursor\":%d}",
                   c.name, c.ocf, c.target_ocf, c.category, c.owner, c.alive ? 1 : 0,
                   c.hostile ? 1 : 0, c.in_crew ? 1 : 0, c.pushing_target ? 1 : 0, c.player,
                   c.x - ObjX, result);
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("game_save_policy");
    {
        using namespace game_save_policy;

        C4GameSaveScenario scenario_plain(false, false);
        C4GameSaveScenario scenario_exact_origin(true, true);
        C4GameSaveSavegame savegame;
        C4GameSaveRecord record_initial(true, true);
        C4GameSaveRecord record_runtime(false, true);
        C4GameSaveRecord record_streaming(false, false);
        C4GameSaveNetwork network_initial(true);
        C4GameSaveNetwork network_runtime(false);

        struct Case
        {
            const char *name;
            C4GameSave *save;
        };

        const Case cases[] = {
            {"scenario", &scenario_plain},
            {"scenario_exact_landscape_and_origin", &scenario_exact_origin},
            {"savegame", &savegame},
            {"record_initial", &record_initial},
            {"record_runtime", &record_runtime},
            {"record_streaming_no_scenario_copy", &record_streaming},
            {"network_initial", &network_initial},
            {"network_runtime", &network_runtime},
        };

        for (const Case &c : cases)
        {
            const Vector v = read(*c.save);
            sep();
            printf("{\"case\":\"%s\",\"save_runtime_data\":%d,\"keep_title\":%d,"
                   "\"save_desc\":%d,\"copy_scenario\":%d,\"create_small_file\":%d,"
                   "\"force_exact_landscape\":%d,\"save_origin\":%d,\"clear_origin\":%d,"
                   "\"save_user_players\":%d,\"save_script_players\":%d,"
                   "\"save_user_player_files\":%d,\"save_script_player_files\":%d,"
                   "\"is_exact\":%d,\"is_synced\":%d,\"sorts\":%d}",
                   c.name, v.save_runtime_data ? 1 : 0, v.keep_title ? 1 : 0,
                   v.save_desc ? 1 : 0, v.copy_scenario ? 1 : 0, v.create_small_file ? 1 : 0,
                   v.force_exact_landscape ? 1 : 0, v.save_origin ? 1 : 0,
                   v.clear_origin ? 1 : 0, v.save_user_players ? 1 : 0,
                   v.save_script_players ? 1 : 0, v.save_user_player_files ? 1 : 0,
                   v.save_script_player_files ? 1 : 0, v.is_exact ? 1 : 0, v.is_synced ? 1 : 0,
                   v.sort_order ? 1 : 0);
        }
    }
    arr_end();
    printf(",\n");

    // C4GameSave::SaveRuntimeData: the ordered component sweep the policy
    // queries above actually drive. Three rules the order encodes:
    //
    //   * Scenario sections are written for an EXACT save only, and Title for
    //     a NON-exact one. The second reads backwards from the first.
    //   * RoundResults is gated on GetSaveUserPlayers(), so the scenario
    //     variant skips it while every exact variant writes it.
    //   * A failing Script/Title/Info write is `nofail` — it logs and the
    //     sweep carries on returning true — while a failing
    //     Landscape/Strings/Objects/Teams write aborts with false. Which side
    //     of that line a component sits on is not visible from its name.
    //
    // The `else` arm that deletes Game.txt/PlayerInfos.txt/SavePlayerInfos.txt
    // is deliberately NOT exercised: it needs
    // `!GetSaveUserPlayers() && !GetSaveScriptPlayers()`, and no shipped
    // variant can produce that. The base returns `IsExact()` for both, so an
    // exact save takes the first arm; and C4GameSaveScenario, the only
    // non-exact one, overrides GetSaveScriptPlayers to a flat true ("script
    // players are also saved; but user players aren't!"). Reaching that arm
    // would need a fabricated sixth variant, which would pin the fixture
    // rather than the engine.
    arr_begin("save_runtime_sequence");
    {
        using namespace game_save_policy;

        struct Case
        {
            const char *name;
            int variant;  // 0 scenario, 1 savegame, 2 record, 3 network
            const char *failing;
        };
        const Case cases[] = {
            {"scenario", 0, nullptr},
            {"savegame", 1, nullptr},
            {"record_runtime", 2, nullptr},
            {"network_runtime", 3, nullptr},
            // Script is nofail: the sweep logs and keeps going.
            {"savegame_script_fails", 1, "Script"},
            // Teams is not: it aborts the whole save.
            {"savegame_teams_fails", 1, "Teams"},
            // And an early abort stops before anything after it.
            {"savegame_landscape_fails", 1, "Landscape"},
        };

        for (const auto &c : cases)
        {
            g_trace.clear();
            g_failing.clear();
            if (c.failing) g_failing.insert(c.failing);

            C4GameSaveScenario scenario(false, false);
            C4GameSaveSavegame savegame;
            C4GameSaveRecord record(false, true);
            C4GameSaveNetwork network(false);
            C4GameSave *save = &scenario;
            if (c.variant == 1) save = &savegame;
            else if (c.variant == 2) save = &record;
            else if (c.variant == 3) save = &network;

            game_save_policy::C4Group group;
            save->pSaveGroup = &group;
            const bool ok = save->SaveRuntimeData();

            sep();
            printf("{\"case\":\"%s\",\"failing\":\"%s\",\"ok\":%s,\"trace\":[",
                   c.name, c.failing ? c.failing : "", ok ? "true" : "false");
            for (size_t i = 0; i < g_trace.size(); i++)
            {
                if (i) printf(",");
                printf("\"%s\"", g_trace[i].c_str());
            }
            printf("]}");
        }
        g_trace.clear();
        g_failing.clear();
    }
    arr_end();
    printf(",\n");

    // The group sort order is one shared constant rather than a per-variant
    // decision, so it is pinned once. It is the component ORDER a saved group
    // is written in, which is what a reader depends on.
    arr_begin("game_save_sort_order");
    {
        game_save_policy::C4GameSaveSavegame savegame;
        sep();
        printf("{\"case\":\"scenario_sort_order\",\"order\":\"%s\"}",
               savegame.GetSortOrder());
    }
    arr_end();
    printf(",\n");

    arr_begin("wildcard_match");
    {
        struct Case
        {
            const char *pattern;
            const char *name;
        };

        const Case cases[] = {
            // Exact and case-insensitive equality.
            {"Scenario.txt", "Scenario.txt"},
            {"scenario.TXT", "Scenario.txt"},
            {"Scenario.txt", "Scenario.tx"},
            {"Scenario.txt", "Scenario.txtx"},
            // The extension patterns every stock sort list is built from.
            {"*.c4d", "Objects.c4d"},
            {"*.c4d", "objects.C4D"},
            {"*.c4d", ".c4d"},
            {"*.c4d", "c4d"},
            {"*.c4d", "Objects.c4dx"},
            // A `*` in the MIDDLE, which needs the backtracking arm.
            {"Loader*.bmp", "Loader.bmp"},
            {"Loader*.bmp", "Loader2.bmp"},
            {"Loader*.bmp", "Loader.bmp.bmp"},
            {"Loader*.bmp", "Loade.bmp"},
            {"Desc*.rtf", "DescDE.rtf"},
            {"Sect*.c4g", "Sect1.c4g"},
            {"StringTbl*.txt", "StringTblUS.txt"},
            // Backtracking that must retry more than once: the first candidate
            // position fails and a later one succeeds.
            {"*ab", "aab"},
            {"*ab", "abab"},
            {"*ab*", "xxabyy"},
            {"a*b*c", "abc"},
            {"a*b*c", "axxbyyc"},
            {"a*b*c", "axxbyy"},
            // Leading, trailing and repeated stars.
            {"*", ""},
            {"*", "anything"},
            {"**", "anything"},
            {"Title*", "Title"},
            {"*Title", "Title"},
            // `?` matches exactly one character and NEVER the end of string.
            {"?", ""},
            {"?", "a"},
            {"?", "ab"},
            {"Icon.?ng", "Icon.png"},
            {"Icon.??g", "Icon.png"},
            {"Icon.???", "Icon.pn"},
            {"Player?.txt", "Player1.txt"},
            {"Player?.txt", "Player.txt"},
            // A literal star in the NAME: the pattern's star matches it, and an
            // exact pattern matches it too.
            {"Wild*.c4d", "Wild*.c4d"},
            {"*.c4d", "Wild*.c4d"},
            // Empty pattern.
            {"", ""},
            {"", "a"},
        };

        for (const Case &c : cases)
        {
            sep();
            printf("{\"pattern\":\"%s\",\"name\":\"%s\",\"match\":%d}", c.pattern, c.name,
                   wildcard::WildcardMatch(c.pattern, c.name) ? 1 : 0);
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("config_language_sequence");
    {
        struct Case
        {
            const char *name;
            const char *source;
        };

        const Case cases[] = {
            // The shipped defaults and the shapes a config file carries.
            {"empty", ""},
            {"single", "DE"},
            {"pair", "DE,US"},
            {"shipped_default", "US,DE"},
            // Whitespace after a separator is skipped outright; whitespace
            // before one is not, and disappears only because the copy stops at
            // two characters.
            {"space_after_comma", "DE, US"},
            {"spaces_everywhere", " DE , US "},
            // Long descriptions are TRUNCATED to two characters rather than
            // rejected, which is the whole point of the condensing pass.
            {"long_names", "German,English"},
            {"mixed_lengths", "DE,English,US"},
            // Empty segments are dropped, so separators do not produce blanks.
            {"empty_segment", "DE,,US"},
            {"leading_comma", ",DE"},
            {"trailing_comma", "DE,"},
            {"only_commas", ",,,"},
            // A single-character code stays one character.
            {"one_char", "D"},
            {"one_char_pair", "D,U"},
            // Case is preserved -- the sequence is copied, not normalized.
            {"lowercase", "de,us"},
            {"mixed_case", "De,uS"},
            // Whitespace-only segments become empty and are dropped.
            {"space_segment", "DE,   ,US"},
            {"only_space", "   "},
        };

        for (const Case &c : cases)
        {
            char target[256] = {0};
            config_language::C4ConfigGeneral general;
            const int count = general.GetLanguageSequence(c.source, target);
            sep();
            printf("{\"case\":\"%s\",\"source\":\"%s\",\"count\":%d,\"target\":\"%s\"}", c.name,
                   c.source, count, target);
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("c4value_operator_equal");
    {
        using namespace c4value_equal;

        // Distinct allocations holding equal content, to show that strings and
        // arrays compare by CONTENT rather than by backing pointer.
        static C4String abc_one{"abc"};
        static C4String abc_two{"abc"};
        static C4String xyz{"xyz"};

        static C4ValueArray one_two{{scalar(C4V_Int, 1), scalar(C4V_Int, 2)}};
        static C4ValueArray one_two_copy{{scalar(C4V_Int, 1), scalar(C4V_Int, 2)}};
        static C4ValueArray one_three{{scalar(C4V_Int, 1), scalar(C4V_Int, 3)}};

        struct Named
        {
            const char *name;
            C4Value value;
        };

        // C4IDs carry only payloads the port can also build: an all-digit id of
        // four or more characters parses numerically, so these are reachable
        // from both sides and let a Bool and a C4ID share a word.
        const Named values[] = {
            {"nil", scalar(C4V_Any, 0)},
            {"int_zero", scalar(C4V_Int, 0)},
            {"int_one", scalar(C4V_Int, 1)},
            {"int_minus_one", scalar(C4V_Int, -1)},
            {"bool_false", scalar(C4V_Bool, 0)},
            {"bool_true", scalar(C4V_Bool, 1)},
            {"c4id_zero", scalar(C4V_C4ID, 0)},
            {"c4id_one", scalar(C4V_C4ID, 1)},
            {"object_zero", scalar(C4V_C4Object, 0)},
            {"object_five", scalar(C4V_C4Object, 5)},
            {"string_abc", string_value(&abc_one)},
            {"string_abc_other_allocation", string_value(&abc_two)},
            {"string_xyz", string_value(&xyz)},
            {"array_one_two", array_value(&one_two)},
            {"array_one_two_other_allocation", array_value(&one_two_copy)},
            {"array_one_three", array_value(&one_three)},
        };

        for (const Named &left : values)
        {
            for (const Named &right : values)
            {
                sep();
                printf("{\"left\":\"%s\",\"right\":\"%s\",\"equal\":%d}", left.name, right.name,
                       left.value == right.value ? 1 : 0);
            }
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("object_change_def");
    {
        struct Case
        {
            const char *name;
            bool contained;
            bool rotateable;
            int32_t start_rotation;
            int32_t blit_mode;      // the object's, before the change
            int32_t new_blit_mode;  // the new definition's
            int32_t color;
            int32_t owner;
            int32_t player_color;
            int32_t new_solid_mask;
            bool has_solid_mask_data;
            bool other_object_has_effects;
            bool unknown_definition;
        };

        const int32_t Custom = container_lifecycle::C4GFXBLIT_CUSTOM;
        const Case cases[] = {
            // An unknown id changes nothing and reports failure.
            {"unknown_definition", false, true, 90, 0, 7, 0, -1, 0, 5, false, false, true},
            // The plain change: counts move, the new definition's state is
            // adopted, and the update chain runs in order.
            {"plain", false, true, 90, 0, 7, 0, -1, 0, 5, false, false, false},
            // Inside a container: out and back with no callbacks either way.
            {"contained_round_trip", true, true, 90, 0, 7, 0, -1, 0, 5, false, false, false},
            // A non-rotateable target zeroes the rotation and its velocity.
            {"non_rotateable_drops_rotation", false, false, 90, 0, 7, 0, -1, 0, 5, false, false,
             false},
            // The blit mode is taken from the definition unless the object set
            // a custom one (C4Object.cpp:1233).
            {"blit_mode_adopted", false, true, 0, 0, 7, 0, -1, 0, 5, false, false, false},
            {"custom_blit_mode_kept", false, true, 0, Custom, 7, 0, -1, 0, 5, false, false,
             false},
            // A colourless object owned by a player picks up that player's
            // colour; one that already has a colour keeps it.
            {"colour_from_player", false, true, 0, 0, 7, 0, 0, 0x334455, 5, false, false, false},
            {"existing_colour_kept", false, true, 0, 0, 7, 0x112233, 0, 0x334455, 5, false,
             false, false},
            // A live solid mask is removed before the definition's replaces it.
            {"solid_mask_replaced", false, true, 0, 0, 7, 0, -1, 0, 5, true, false, false},
            // Every object's effects are told, not just this one's.
            {"effects_told", false, true, 0, 0, 7, 0, -1, 0, 5, false, true, false},
        };

        for (const Case &c : cases)
        {
            container_lifecycle::DefStub old_def, new_def;
            old_def.id = 100;
            old_def.Count = 4;
            old_def.Rotateable = true;
            new_def.id = 200;
            new_def.Count = 1;
            new_def.Rotateable = c.rotateable;
            new_def.BlitMode = c.new_blit_mode;
            new_def.SolidMask = c.new_solid_mask;
            container_lifecycle::g_definitions[0] = &old_def;
            container_lifecycle::g_definitions[1] = &new_def;

            container_lifecycle::PlayerColorStub player;
            player.ColorDw = c.player_color;
            container_lifecycle::g_player_colors.Held = &player;

            container_lifecycle::C4Object object;
            object.Tag = "object";
            object.Def = &old_def;
            object.id = old_def.id;
            object.r = c.start_rotation;
            object.fix_r = itofix(c.start_rotation);
            object.rdir = itofix(1);
            object.BlitMode = c.blit_mode;
            object.Color = c.color;
            object.Owner = c.owner;
            object.Action.Act = 0;
            // The lifted body `delete`s the mask, so it has to be on the heap.
            if (c.has_solid_mask_data)
                object.pSolidMaskData = new container_lifecycle::C4Object::SolidMaskDataStub();

            container_lifecycle::C4Object container;
            container.Tag = "container";
            container.Def = &old_def;
            if (c.contained)
            {
                object.Contained = &container;
                container.Contents.Add(&object, container_lifecycle::C4ObjectList::stContents);
            }

            // A second object whose effects must hear about the change.
            container_lifecycle::C4Object other;
            other.Tag = "other";
            other.Def = &old_def;
            container_lifecycle::C4Object::EffectListStub other_effects;
            if (c.other_object_has_effects) other.pEffects = &other_effects;
            container_lifecycle::C4ObjectLink other_link{&other, nullptr};
            container_lifecycle::C4ObjectLink object_link{&object, &other_link};
            container_lifecycle::ChangeDefGame.Objects.First = &object_link;

            container_lifecycle::g_config_count = 0;
            container_lifecycle::g_call_count = 0;

            const bool changed = object.ChangeDef(c.unknown_definition ? 999 : new_def.id);

            sep();
            printf("{\"case\":\"%s\",\"changed\":%d,\"id\":%d,\"old_count\":%d,"
                   "\"new_count\":%d,\"rotation\":%d,\"rdir\":%d,\"blit_mode\":%d,"
                   "\"colour\":%d,\"solid_mask\":%d,\"unsorted\":%d,\"contained\":%d,"
                   "\"calls\":[",
                   c.name, changed ? 1 : 0, object.id, old_def.Count, new_def.Count, object.r,
                   object.rdir.val, object.BlitMode, object.Color, object.SolidMask,
                   object.Unsorted ? 1 : 0, object.Contained == &container ? 1 : 0);
            for (int32_t i = 0; i < container_lifecycle::g_call_count
                                && i < container_lifecycle::MaxCalls;
                 ++i)
                printf("%s\"%s\"", i ? "," : "", container_lifecycle::g_calls[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("object_death");
    {
        struct Case
        {
            const char *name;
            int32_t alive;
            bool forced;
            bool has_effects;
            bool effects_resurrect;
            bool has_info;
            bool has_player;
            bool in_view_list;
            int32_t category;
            int32_t cause_player;
            int32_t contents;
        };

        const Case cases[] = {
            // Not alive: nothing happens at all.
            {"already_dead", 0, false, true, false, true, true, false, object_removal::C4D_Living, 3, 2},
            // The plain death, with the cause player reaching the callback.
            {"plain", 1, false, false, false, false, false, false, object_removal::C4D_Living, 3, 0},
            // An effect clear that resurrects aborts an unforced kill — after
            // the clear has already run.
            {"resurrected_aborts", 1, false, true, true, false, false, false, object_removal::C4D_Living, 3, 0},
            // ...but not a forced one.
            {"resurrected_forced_continues", 1, true, true, true, false, false, false,
             object_removal::C4D_Living, 3, 0},
            // An effect clear that does not resurrect just runs.
            {"effects_cleared", 1, false, true, false, false, false, false, object_removal::C4D_Living, 3, 0},
            // The info bookkeeping: died, death count, retire.
            {"with_info", 1, false, false, false, true, false, false, object_removal::C4D_Living, 3, 0},
            // Contents are EXITED, not removed — a dying Clonk drops its load.
            {"contents_exited", 1, false, false, false, false, false, false, object_removal::C4D_Living, 3, 2},
            // A living object already in the player's fog-of-war view list
            // keeps its view range; anything else has it reset.
            {"living_in_view_keeps_range", 1, false, false, false, false, true, true,
             object_removal::C4D_Living, 3, 0},
            {"living_out_of_view_resets", 1, false, false, false, false, true, false,
             object_removal::C4D_Living, 3, 0},
            {"non_living_resets_range", 1, false, false, false, false, true, true, 0, 3, 0},
            {"no_player_resets_range", 1, false, false, false, false, false, false, object_removal::C4D_Living,
             3, 0},
        };

        for (const Case &c : cases)
        {
            object_removal::DefStub object_def, content_def;
            object_removal::EffectsStub *effects =
                c.has_effects ? new object_removal::EffectsStub() : nullptr;
            if (effects) effects->Resurrects = c.effects_resurrect;
            object_removal::InfoStub info;
            object_removal::C4Player player;
            player.FoWViewObjs.Contains = c.in_view_list;
            object_removal::Game.Players.Held = c.has_player ? &player : nullptr;

            object_removal::C4Object object;
            object.Tag = "object";
            object.Def = &object_def;
            object.Alive = c.alive;
            object.Select = 1;
            object.Category = c.category;
            object.Owner = c.has_player ? 0 : -1;
            object.LastEnergyLossCausePlayer = c.cause_player;
            object.pEffects = effects;
            object.Info = c.has_info ? &info : nullptr;

            object_removal::C4Object contents[2];
            for (int32_t i = 0; i < c.contents; ++i)
            {
                contents[i].Tag = "content";
                contents[i].Def = &content_def;
                contents[i].Contained = &object;
                object.Contents.Add(&contents[i]);
            }

            object_removal::g_config_count = 0;
            object_removal::g_trace_count = 0;
            object_removal::g_removing = &object;

            object.AssignDeath(c.forced);

            sep();
            printf("{\"case\":\"%s\",\"forced\":%d,\"alive_after\":%d,\"select_after\":%d,"
                   "\"death_player_seen\":%d,\"has_died\":%d,\"death_count\":%d,"
                   "\"contents_left\":%d,\"contents_contained\":%d,\"calls\":[",
                   c.name, c.forced ? 1 : 0, object.Alive, object.Select,
                   object.DeathPlayerSeen, info.HasDied ? 1 : 0, info.DeathCount,
                   object.Contents.Count(),
                   c.contents ? (contents[0].Contained == &object ? 1 : 0) : 0);
            for (int32_t i = 0; i < object_removal::g_trace_count
                                && i < object_removal::MaxTrace;
                 ++i)
                printf("%s\"%s\"", i ? "," : "", object_removal::g_trace[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("object_removal");
    {
        using namespace object_removal;

        struct Case
        {
            const char *name;
            bool contained;
            bool has_effects;
            bool has_particles;
            bool has_info;
            bool has_reference;
            bool inactive;
            int32_t contents;
            bool exit_contents;
            int32_t already_removed;
            int32_t config_count;
            CallConfig configs[MaxConfigs];
        };

        const Case cases[] = {
            // The bare teardown, with nothing attached.
            {"plain", false, false, false, false, false, false, 0, false, 0, 0, {}},
            // Already deleted: the very first check returns and nothing runs.
            {"already_deleted", false, true, true, true, true, false, 2, false, 1, 0, {}},
            // Contained: the container's ContentsDestruction comes FIRST, and
            // the object leaves the container only at the end.
            {"contained", true, false, false, false, false, false, 0, false, 0, 0, {}},
            // A ContentsDestruction that deletes the object stops everything
            // else, including the object's own Destruction.
            {"contents_destruction_deletes", true, true, false, false, false, false, 1, false, 0,
             1, {{"container", PSF_ContentsDestruction, Effect::RemoveRemoving, false}}},
            // Destruction deleting the object stops the effect clear.
            {"destruction_deletes", false, true, false, false, false, false, 1, false, 0, 1,
             {{"object", PSF_Destruction, Effect::RemoveRemoving, false}}},
            // Effects are cleared after Destruction, and the particles after
            // that.
            {"effects_and_particles", false, true, true, false, false, false, 0, false, 0, 0, {}},
            // An inactive object is put back on the main list before it is
            // deleted (C4Object.cpp:277-283).
            {"inactive_reactivated_first", false, false, false, false, false, true, 0, false, 0,
             0, {}},
            // Contents: removed recursively by default, so each one runs its
            // own teardown...
            {"contents_removed_recursively", false, false, false, false, false, false, 2, false,
             0, 0, {}},
            // ...or Exited when the caller asks, which spills them instead.
            {"contents_exited", false, false, false, false, false, false, 2, true, 0, 0, {}},
            // Contained WITH contents: the cargo is torn down before this
            // object leaves its own container.
            {"contained_with_contents", true, false, false, false, false, false, 2, false, 0, 0,
             {}},
            // The info retire and reference/pointer cleanup tail.
            {"info_and_references", false, false, false, true, true, false, 0, false, 0, 0, {}},
        };

        for (const Case &c : cases)
        {
            DefStub object_def, container_def, content_def;
            // The lifted teardown `delete`s the effect list, so it has to be
            // heap-allocated.
            EffectsStub *effects = c.has_effects ? new EffectsStub() : nullptr;
            InfoStub info;

            object_removal::C4Object object;
            object.Tag = "object";
            object.Def = &object_def;
            object.Status = c.already_removed ? C4OS_DELETED
                                              : (c.inactive ? C4OS_INACTIVE : C4OS_NORMAL);
            object.pEffects = effects;
            object.Info = c.has_info ? &info : nullptr;
            object.FrontParticles.Present = c.has_particles;
            object.BackParticles.Present = c.has_particles;
            object.x = 40;
            object.y = 50;

            object_removal::C4Object::RefStub reference;
            g_ref_owner = &object;
            if (c.has_reference) object.FirstRef = &reference;

            object_removal::C4Object container;
            container.Tag = "container";
            container.Def = &container_def;
            if (c.contained)
            {
                object.Contained = &container;
                container.Contents.Add(&object);
            }

            object_removal::C4Object contents[2];
            for (int32_t i = 0; i < c.contents; ++i)
            {
                contents[i].Tag = "content";
                contents[i].Def = &content_def;
                contents[i].Contained = &object;
                object.Contents.Add(&contents[i]);
            }

            g_config_count = c.config_count;
            for (int32_t i = 0; i < c.config_count; ++i) g_configs[i] = c.configs[i];
            g_trace_count = 0;
            g_removing = &object;

            object.AssignRemoval(c.exit_contents);

            sep();
            printf("{\"case\":\"%s\",\"status\":%d,\"removal_delay\":%d,"
                   "\"still_contained\":%d,\"container_contents\":%d,\"own_contents\":%d,"
                   "\"def_count\":%d,\"content_status\":[%d,%d],\"calls\":[",
                   c.name, object.Status, object.RemovalDelay,
                   object.Contained == &container ? 1 : 0, container.Contents.Count(),
                   object.Contents.Count(), object_def.Count, contents[0].Status,
                   contents[1].Status);
            for (int32_t i = 0; i < g_trace_count && i < MaxTrace; ++i)
                printf("%s\"%s\"", i ? "," : "", g_trace[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("effect_execute");
    {
        struct Row
        {
            int32_t priority; // zero marks it already dead
            int32_t interval;
            bool has_timer;
            int32_t timer_result;
            int32_t start_time;
        };
        struct Case
        {
            const char *name;
            int32_t frames;
            Row rows[3];
        };
        const int32_t Kill = effect_check::C4Fx_Execute_Kill;
        const int32_t OK = effect_check::C4Fx_OK;

        const Case cases[] = {
            // An interval of zero never fires the timer, however long it runs.
            {"interval_zero_never_fires", 4,
             {{100, 0, true, OK, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            // Interval 2 fires on the even frames only, and the clock is
            // advanced BEFORE the modulo, so frame 1 already counts as time 1.
            {"interval_two_fires_every_other", 4,
             {{100, 2, true, OK, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            {"interval_one_fires_every_frame", 3,
             {{100, 1, true, OK, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            // A non-zero start time shifts which frames land on the boundary.
            {"start_time_shifts_boundary", 4,
             {{100, 3, true, OK, 1}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            // The timer answering Kill finishes the effect, which the NEXT
            // frame's pass then unlinks.
            {"timer_kills_then_unlinks", 3,
             {{100, 1, true, Kill, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            // An interval with no timer function is killed the moment the
            // boundary arrives (C4Effect.cpp:355-357).
            {"interval_without_timer_dies", 3,
             {{100, 2, false, OK, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            // Already-dead effects are unlinked on the first pass, wherever
            // they sit in the list.
            {"dead_head_unlinked", 2,
             {{100, 0, true, OK, 0}, {60, 0, true, OK, 0}, {0, 0, true, OK, 0}}},
            {"dead_middle_unlinked", 2,
             {{100, 0, true, OK, 0}, {0, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            {"dead_tail_unlinked", 2,
             {{0, 0, true, OK, 0}, {60, 0, true, OK, 0}, {20, 0, true, OK, 0}}},
            {"all_dead_unlinked", 2,
             {{0, 0, true, OK, 0}, {0, 0, true, OK, 0}, {0, 0, true, OK, 0}}},
        };

        for (const Case &c : cases)
        {
            // Same list shape as the check section: added A, B, C, kept sorted
            // by ascending priority, so the pass visits C, then B, then A.
            const char *names[3] = {"EffectA", "EffectB", "EffectC"};
            const int32_t order[3] = {2, 1, 0};
            effect_check::FnEffect checkers[3];
            effect_check::FnTimer timers[3];
            effect_check::C4Effect *effects[3];
            for (int32_t i = 0; i < 3; ++i)
            {
                effects[i] = new effect_check::C4Effect();
                effects[i]->Name = names[i];
                effects[i]->iPriority = c.rows[i].priority;
                effects[i]->iNumber = i + 1;
                effects[i]->iIntervall = c.rows[i].interval;
                effects[i]->iTime = c.rows[i].start_time;
                effects[i]->TimerResult = c.rows[i].timer_result;
                checkers[i].owner = effects[i];
                timers[i].owner = effects[i];
                effects[i]->pFnEffect = &checkers[i];
                effects[i]->pFnTimer = c.rows[i].has_timer ? &timers[i] : nullptr;
            }
            for (int32_t i = 0; i < 3; ++i)
                effects[order[i]]->pNext = i + 1 < 3 ? effects[order[i + 1]] : nullptr;

            effect_check::C4Object object;
            object.pEffects = effects[order[0]];
            effect_check::g_trace_count = 0;
            effect_check::g_deleted = 0;

            sep();
            printf("{\"case\":\"%s\",\"frames\":%d,\"passes\":[", c.name, c.frames);
            for (int32_t frame = 0; frame < c.frames; ++frame)
            {
                const int32_t before = effect_check::g_trace_count;
                if (object.pEffects) object.pEffects->Execute(&object);
                if (frame) printf(",");
                printf("{\"frame\":%d,\"deleted\":%d,\"live\":[", frame,
                       effect_check::g_deleted);
                {
                    bool first_live = true;
                    for (effect_check::C4Effect *live = object.pEffects; live; live = live->pNext)
                    {
                        printf("%s\"%s\"", first_live ? "" : ",", live->Name);
                        first_live = false;
                    }
                }
                printf("],\"calls\":[");
                for (int32_t i = before; i < effect_check::g_trace_count
                                         && i < effect_check::MaxTrace;
                     ++i)
                    printf("%s\"%s\"", i > before ? "," : "", effect_check::g_trace[i]);
                printf("]}");
            }
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("effect_check");
    {
        struct Case
        {
            const char *name;
            int32_t priority;   // the incoming effect's priority
            int32_t results[3]; // what each existing effect's checker answers
            int32_t add_result; // what the absorbing effect's FxAdd returns
            bool dead[3];
            bool has_function[3];
        };
        const int32_t OK = effect_check::C4Fx_OK;
        const int32_t Deny = effect_check::C4Fx_Effect_Deny;
        const int32_t Annul = effect_check::C4Fx_Effect_Annul;
        const int32_t AnnulCalls = effect_check::C4Fx_Effect_AnnulCalls;
        const int32_t StartDeny = effect_check::C4Fx_Start_Deny;

        const Case cases[] = {
            // Priority 1 is always allowed and asks nobody (C4Effect.cpp:274).
            {"priority_one_asks_nobody", 1, {Deny, Deny, Deny}, OK, {false, false, false},
             {true, true, true}},
            // Nobody objects: every checker of at least the new priority runs,
            // in list order, and the answer is zero.
            {"all_accept", 50, {OK, OK, OK}, OK, {false, false, false}, {true, true, true}},
            // A Deny short-circuits the whole walk.
            {"first_denies", 50, {Deny, OK, OK}, OK, {false, false, false}, {true, true, true}},
            {"second_denies", 50, {OK, Deny, OK}, OK, {false, false, false}, {true, true, true}},
            // Effects BELOW the new priority are never asked, so a low-priority
            // denier cannot stop it.
            {"low_priority_denier_ignored", 150, {Deny, Deny, Deny}, OK,
             {false, false, false}, {true, true, true}},
            // Dead effects and effects without a callback are skipped.
            {"dead_effect_skipped", 50, {Deny, OK, OK}, OK, {true, false, false},
             {true, true, true}},
            {"functionless_effect_skipped", 50, {Deny, OK, OK}, OK, {false, false, false},
             {false, true, true}},
            // An Annul nominates its effect to absorb the newcomer; the walk
            // CONTINUES, and the LAST annulling effect wins.
            {"annul_absorbs", 50, {Annul, OK, OK}, OK, {false, false, false},
             {true, true, true}},
            // At priority 20 every effect is asked, so the LAST annulling one
            // is the absorber — its number is what comes back, not the first's.
            {"last_annul_wins", 20, {Annul, OK, Annul}, OK, {false, false, false},
             {true, true, true}},
            // A Deny after an Annul still wins outright.
            {"deny_after_annul", 50, {Annul, Deny, OK}, OK, {false, false, false},
             {true, true, true}},
            // AnnulCalls brackets the FxAdd in temp remove/readd of the
            // effects above the absorber.
            {"annul_calls_brackets_add", 50, {AnnulCalls, OK, OK}, OK,
             {false, false, false}, {true, true, true}},
            // The same on the LAST effect, which has nothing above it: no
            // bracket (C4Effect.cpp:298,303 both test pNext).
            {"annul_calls_on_last_effect", 20, {OK, OK, AnnulCalls}, OK,
             {false, false, false}, {true, true, true}},
            // An FxAdd that denies kills the absorbing effect and reports
            // Annul rather than its number.
            {"add_denies_kills_absorber", 50, {Annul, OK, OK}, StartDeny,
             {false, false, false}, {true, true, true}},
            {"annul_calls_add_denies", 50, {AnnulCalls, OK, OK}, StartDeny,
             {false, false, false}, {true, true, true}},
        };

        for (const Case &c : cases)
        {
            // A, B and C are added in that order — so those are their effect
            // NUMBERS — but the engine keeps its list sorted by ascending
            // priority, so the walk visits C, then B, then A.
            effect_check::FnEffect functions[3];
            effect_check::C4Effect effects[3];
            const char *names[3] = {"EffectA", "EffectB", "EffectC"};
            const int32_t priorities[3] = {100, 60, 20};
            const int32_t order[3] = {2, 1, 0}; // C, B, A
            for (int32_t i = 0; i < 3; ++i)
            {
                effects[i].Name = names[i];
                effects[i].iPriority = c.dead[i] ? 0 : priorities[i];
                effects[i].iNumber = i + 1;
                effects[i].EffectResult = c.results[i];
                effects[i].AddResult = c.add_result;
                functions[i].owner = &effects[i];
                effects[i].pFnEffect = c.has_function[i] ? &functions[i] : nullptr;
            }
            for (int32_t i = 0; i < 3; ++i)
                effects[order[i]].pNext = i + 1 < 3 ? &effects[order[i + 1]] : nullptr;

            effect_check::g_trace_count = 0;
            effect_check::C4Value none;
            const int32_t result = effects[order[0]].Check(
                nullptr, "Newcomer", c.priority, 35, none, none, none, none, false);

            sep();
            printf("{\"case\":\"%s\",\"priority\":%d,\"result\":%d,\"killed\":[%d,%d,%d],"
                   "\"trace\":[",
                   c.name, c.priority, result, effects[0].Killed ? 1 : 0,
                   effects[1].Killed ? 1 : 0, effects[2].Killed ? 1 : 0);
            for (int32_t i = 0; i < effect_check::g_trace_count && i < effect_check::MaxTrace; ++i)
                printf("%s\"%s\"", i ? "," : "", effect_check::g_trace[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("container_lifecycle");
    {
        using namespace container_lifecycle;

        struct Case
        {
            const char *name;
            const char *op;      // enter | exit | collect
            bool self_target;    // enter into itself
            bool null_target;
            bool start_contained;
            bool recursive;      // target sits inside the object
            bool want_reject_flag;
            bool copy_motion;
            bool refuse_add;
            bool living;
            int32_t base;
            int32_t base_functionality;
            int32_t rules;
            int32_t object_id;
            const char *action_name;
            uint32_t ocf;
            int32_t config_count;
            CallConfig configs[MaxConfigs];
        };

        const Case cases[] = {
            // --- Enter -----------------------------------------------------
            {"enter_null_target", "enter", false, true, false, false, false, false, false, false,
             -1, 0, 0, 0, "", 0, 0, {}},
            {"enter_self", "enter", true, false, false, false, false, false, false, false, -1, 0,
             0, 0, "", 0, 0, {}},
            {"enter_rejected", "enter", false, false, false, false, false, false, false, false,
             -1, 0, 0, 0, "", 0, 1,
             {{"object", PSF_RejectEntrance, 1, Effect::None}}},
            // The target already sits inside the object, so entering it would
            // close a loop; the guard runs AFTER RejectEntrance.
            {"enter_recursive", "enter", false, false, false, true, false, false, false, false,
             -1, 0, 0, 0, "", 0, 0, {}},
            // RejectCollection is consulted only when the caller asked for the
            // flag — which C4Object::Collect does and a plain script Enter does
            // not — so it is exercised through the collect cases below.
            {"enter_plain", "enter", false, false, false, false, false, false, false, false, -1,
             0, 0, 0, "", 0, 0, {}},
            // fCopyMotion inserts the solid-mask removal and the motion copy
            // BEFORE the OCF refresh (C4Object.cpp:1614-1620).
            {"enter_copy_motion", "enter", false, false, false, false, false, true, false, false,
             -1, 0, 0, 0, "", 0, 0, {}},
            // A living object keeps its own controller; anything else inherits
            // the container's (C4Object.cpp:1608-1609).
            {"enter_living_keeps_controller", "enter", false, false, false, false, false, false,
             false, true, -1, 0, 0, 0, "", 0, 0, {}},
            // Already contained: Exit runs first, with its own two callbacks.
            {"enter_from_container", "enter", false, false, true, false, false, false, false,
             false, -1, 0, 0, 0, "", 0, 0, {}},
            // Collection2 removing the object abandons the Entrance call.
            {"enter_collection2_kills", "enter", false, false, false, false, false, false, false,
             false, -1, 0, 0, 0, "", 0, 1,
             {{"target", PSF_Collection2, 0, Effect::ExitEntering}}},
            // The re-check after Entrance tests the CONTAINER's status, not the
            // entering object's — so an Entrance that removes the object itself
            // does NOT stop the auto-sell tail (C4Object.cpp:1629-1633).
            {"enter_entrance_clears_own_status", "enter", false, false, false, false, false,
             false, false, false, 3, BASEFUNC_AutoSellContents, 0, 0, "", 0, 1,
             {{"object", PSF_Entrance, 0, Effect::ClearSelfStatus}}},
            // Removing the CONTAINER is what the re-check catches.
            {"enter_entrance_clears_container", "enter", false, false, false, false, false, false,
             false, false, 3, BASEFUNC_AutoSellContents, 0, 0, "", 0, 1,
             {{"object", PSF_Entrance, 0, Effect::ExitEntering}}},
            // A valid base plus the realism bit runs the auto-sell tail.
            {"enter_auto_sell", "enter", false, false, false, false, false, false, false, false,
             3, BASEFUNC_AutoSellContents, 0, 0, "", 0, 0, {}},
            {"enter_base_without_realism", "enter", false, false, false, false, false, false,
             false, false, 3, 0, 0, 0, "", 0, 0, {}},

            // --- Exit ------------------------------------------------------
            {"exit_not_contained", "exit", false, false, false, false, false, false, false, false,
             -1, 0, 0, 0, "", 0, 0, {}},
            {"exit_plain", "exit", false, false, true, false, false, false, false, false, -1, 0,
             0, 0, "", 0, 0, {}},
            // Departure putting the object back in a container makes Exit
            // report failure even though it did everything (C4Object.cpp:1563).
            {"exit_reentered_by_script", "exit", false, false, true, false, false, false, false,
             false, -1, 0, 0, 0, "", 0, 1,
             {{"object", PSF_Departure, 0, Effect::ReEnter}}},

            // --- Collect ---------------------------------------------------
            // Collect's FlyBase flag gate is a pure decision the port factors
            // out as `flag_collection_blocked`, and it keeps its existing
            // Rust-side coverage; driving it here would need a FLAG definition
            // with a FlyBase action map and the cached rule, none of which this
            // fixture models.
            {"collect_plain", "collect", false, false, false, false, false, false, false, false,
             -1, 0, 0, 0, "", 0, 0, {}},
            // The three hit calls are gated on their own OCF bits and run in
            // order (C4Object.cpp:5710-5712).
            {"collect_hit_speeds", "collect", false, false, false, false, false, false, false,
             false, -1, 0, 0, 0, "",
             OCF_HitSpeed1 | OCF_HitSpeed2 | OCF_HitSpeed3, 0, {}},
            // A Hit callback that removes the object skips the rest.
            {"collect_hit_kills", "collect", false, false, false, false, false, false, false,
             false, -1, 0, 0, 0, "", OCF_HitSpeed1 | OCF_HitSpeed2, 1,
             {{"object", PSF_Hit, 0, Effect::ClearSelfStatus}}},
            // A refused Enter stops Collect before CancelAttach.
            {"collect_enter_refused", "collect", false, false, false, false, false, false, false,
             false, -1, 0, 0, 0, "", 0, 1,
             {{"object", PSF_RejectEntrance, 1, Effect::None}}},
            // The container's own refusal reports through the RejectCollect
            // flag, and Collect turns that into a plain failure.
            {"collect_rejected_by_container", "collect", false, false, false, false, false, false,
             false, false, -1, 0, 0, 0, "", 0, 1,
             {{"target", PSF_RejectCollection, 1, Effect::None}}},
        };

        for (const Case &c : cases)
        {
            DefStub object_def;
            object_def.id = c.object_id;
            object_def.ActMap[0].Name = c.action_name;
            DefStub target_def;
            DefStub outside_def;

            container_lifecycle::C4Object object;
            object.Tag = "object";
            object.Def = &object_def;
            object.Controller = 5;
            object.Base = c.base;
            object.OCF = c.ocf;
            object.Alive = c.living ? 1 : 0;
            object.Category = c.living ? C4D_Living : 0;
            if (c.action_name[0]) object.Action.Act = 0;

            container_lifecycle::C4Object target;
            target.Tag = "target";
            target.Def = &target_def;
            target.Controller = 9;
            target.Base = c.base;
            target.Contents.RefuseAdd = c.refuse_add;

            container_lifecycle::C4Object outside;
            outside.Tag = "outside";
            outside.Def = &outside_def;
            outside.Controller = 2;

            g_reenter_target = &outside;
            g_entering_object = &object;
            Game.Rules = c.rules;
            Game.C4S.Game.Realism.BaseFunctionality = c.base_functionality;
            g_config_count = c.config_count;
            for (int32_t i = 0; i < c.config_count; ++i) g_configs[i] = c.configs[i];
            g_call_count = 0;

            if (c.start_contained)
            {
                object.Contained = &outside;
                outside.Contents.Add(&object, container_lifecycle::C4ObjectList::stContents);
            }
            if (c.recursive)
            {
                target.Contained = &object;
                object.Contents.Add(&target, container_lifecycle::C4ObjectList::stContents);
            }

            bool reject_collect = false;
            bool result = false;
            if (SEqual(c.op, "enter"))
            {
                container_lifecycle::C4Object *destination = c.null_target ? nullptr : (c.self_target ? &object : &target);
                result = object.Enter(
                    destination, true, c.copy_motion,
                    c.want_reject_flag ? &reject_collect : nullptr);
            }
            else if (SEqual(c.op, "exit"))
            {
                result = object.Exit(11, 22, 33, itofix(1), itofix(2), itofix(3) / 10, true);
            }
            else
            {
                result = target.Collect(&object);
            }

            sep();
            printf("{\"case\":\"%s\",\"op\":\"%s\",\"result\":%d,\"reject_collect\":%d,"
                   "\"contained_is_target\":%d,\"contained_is_outside\":%d,"
                   "\"target_contents\":%d,\"controller\":%d,\"mobile\":%d,"
                   "\"in_liquid\":%d,\"x\":%d,\"y\":%d,\"r\":%d,\"xdir\":%d,"
                   "\"ydir\":%d,\"rdir\":%d,\"calls\":[",
                   c.name, c.op, result ? 1 : 0, reject_collect ? 1 : 0,
                   object.Contained == &target ? 1 : 0, object.Contained == &outside ? 1 : 0,
                   target.Contents.Count, object.Controller, object.Mobile, object.InLiquid,
                   object.x, object.y, object.r, object.xdir.val, object.ydir.val,
                   object.rdir.val);
            for (int32_t i = 0; i < g_call_count && i < MaxCalls; ++i)
                printf("%s\"%s\"", i ? "," : "", g_calls[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("target_bounds");
    {
        struct Case
        {
            const char *name;
            int32_t target;
            int32_t low, high;
            int32_t cnat_low, cnat_hi;
            int32_t xdir_raw, ydir_raw;
        };
        const Case cases[] = {
            {"inside", 50, 0, 100, CNAT_Left, CNAT_Right, 65536, -65536},
            {"below_left", -10, 0, 100, CNAT_Left, CNAT_Right, 65536, -65536},
            {"above_right", 140, 0, 100, CNAT_Left, CNAT_Right, 65536, -65536},
            // The vertical pair clears ydir instead.
            {"below_top", -10, 0, 100, CNAT_Top, CNAT_Bottom, 65536, -65536},
            {"above_bottom", 140, 0, 100, CNAT_Top, CNAT_Bottom, 65536, -65536},
            // The comparisons are strict, so sitting exactly on a limit is not
            // a bound crossing.
            {"exactly_low", 0, 0, 100, CNAT_Left, CNAT_Right, 65536, -65536},
            {"exactly_high", 100, 0, 100, CNAT_Left, CNAT_Right, 65536, -65536},
            // Crossed limits: clamping to low puts the target above high, so
            // the second arm fires too and the low contact is reported first.
            {"crossed_limits", 50, 80, 20, CNAT_Left, CNAT_Right, 65536, -65536},
        };

        for (const Case &c : cases)
        {
            shape_contact::C4Object object;
            object.xdir.val = c.xdir_raw;
            object.ydir.val = c.ydir_raw;
            int32_t target = c.target;
            object.TargetBounds(target, c.low, c.high, c.cnat_low, c.cnat_hi);

            sep();
            printf("{\"case\":\"%s\",\"target\":%d,\"low\":%d,\"high\":%d,"
                   "\"cnat_low\":%d,\"cnat_hi\":%d,\"xdir_before\":%d,\"ydir_before\":%d,"
                   "\"bounded\":%d,\"xdir_after\":%d,\"ydir_after\":%d,\"contacts\":[",
                   c.name, c.target, c.low, c.high, c.cnat_low, c.cnat_hi, c.xdir_raw, c.ydir_raw,
                   target, object.xdir.val, object.ydir.val);
            for (int32_t i = 0; i < object.ContactCallCount; ++i)
                printf("%s%d", i ? "," : "", object.ContactCalls[i]);
            printf("]}");
        }
    }
    arr_end();
    printf(",\n");

    arr_begin("itofix");
    const int xs[] = {0, 1, -1, 7, -7, 15, 100, -100, 32767, -32768, 12345};
    const int precs[] = {1, 10, 100, 256, 1000};
    for (int x : xs)
    {
        sep();
        printf("{\"x\":%d,\"prec\":1,\"raw\":%d}", x, itofix(x).val);
        for (int p : precs)
        {
            sep();
            printf("{\"x\":%d,\"prec\":%d,\"raw\":%d}", x, p, itofix(x, p).val);
        }
    }
    arr_end();
    printf(",\n");

    // 2. fixtoi: rounding back to integer (whole and precision-multiplied).
    arr_begin("fixtoi");
    const int raws[] = {0, 65536, -65536, 98304, -98304, 32768, -32768, 32767, 33000, 327980, 70000};
    for (int r : raws)
    {
        C4Fixed f;
        f.val = r; // raw assignment via the (public) val field
        sep();
        printf("{\"raw\":%d,\"prec\":1,\"result\":%d}", r, fixtoi(f));
        for (int p : precs)
        {
            sep();
            printf("{\"raw\":%d,\"prec\":%d,\"result\":%d}", r, p, fixtoi(f, p));
        }
    }
    arr_end();
    printf(",\n");

    // 3. arithmetic: +, -, *, / on fixed operands (velocity scaling, etc.).
    arr_begin("arith");
    struct Pair { int a, b; };
    const Pair pairs[] = {{3, 4}, {7, 3}, {12, 4}, {5, 3}, {-6, 2}, {100, 7}, {1, 256}};
    for (auto pr : pairs)
    {
        C4Fixed a = itofix(pr.a), b = itofix(pr.b);
        sep();
        printf("{\"a\":%d,\"b\":%d,\"add\":%d,\"sub\":%d,\"mul\":%d,\"div\":%d}",
               pr.a, pr.b, (a + b).val, (a - b).val, (a * b).val, (a / b).val);
    }
    // FIXED100/FIXED256/FIXED10 helper constants used by physics.
    sep();
    printf("{\"fixed100_10\":%d,\"fixed256_10\":%d,\"fixed10_10\":%d}",
           FIXED100(10).val, FIXED256(10).val, FIXED10(10).val);
    arr_end();
    printf(",\n");

    // 4. trig: Sin/Cos at representative degrees (rotation, SimFlight).
    arr_begin("trig");
    const int degs[] = {0, 30, 45, 60, 90, 120, 135, 180, 225, 270, 315, 359, -45, -90};
    for (int d : degs)
    {
        sep();
        printf("{\"deg\":%d,\"sin\":%d,\"cos\":%d}", d, Sin(itofix(d)).val, Cos(itofix(d)).val);
    }
    arr_end();
    printf(",\n");

    // 4b. FnSin/FnCos with omitted radius. C4Script.cpp:3224-3238 only
    // defaults precision; the zero-filled radius therefore reaches fixtoi.
    arr_begin("script_trig_default_radius");
    const int script_degs[] = {0, 30, 90, 180, 270, 359, -45};
    for (int d : script_degs)
    {
        sep();
        printf("{\"deg\":%d,\"sin\":%d,\"cos\":%d}",
               d, fixtoi(Sin(itofix(d)), 0), fixtoi(Cos(itofix(d)), 0));
    }
    arr_end();
    printf(",\n");

    // 4c. FnSqrt over the whole domain shape: negatives, small exact and
    // inexact roots, and the top of the int32_t range where `iSqrt * iSqrt`
    // wraps and the correcting decrement is skipped. 2147395600 is 46340^2,
    // the last input whose result is plain floor(sqrt).
    arr_begin("script_sqrt");
    const int sqrt_inputs[] = {
        -2147483647 - 1, -100, -1, 0, 1, 2, 3, 4, 8, 9, 15, 16, 24, 25,
        99, 100, 101, 65535, 65536, 1000000, 1073741823,
        2147395599, 2147395600, 2147395601, 2147450880,
        2147483646, 2147483647,
    };
    for (int v : sqrt_inputs)
    {
        sep();
        printf("{\"value\":%d,\"root\":%d}", v,
               effect_position_oracle::FnSqrt(nullptr, v));
    }
    arr_end();
    printf(",\n");

    // 5. RNG: the C++ LCG. FixedRandom(seed) then Random(range) sequence.
    {
        const uint32_t seed = 12345;
        FixedRandom(seed);
        printf("\"rng_random\":{\"seed\":%u,\"sequence\":[", seed);
        for (int i = 0; i < 64; i++)
        {
            if (i) printf(",");
            int range = (i % 4 == 0) ? 100 : (i % 4 == 1) ? 6 : (i % 4 == 2) ? 1000 : 2;
            printf("{\"range\":%d,\"val\":%d}", range, Random(range));
        }
        // RandomCount must advance once per call (incl. range==0 sync semantics).
        printf("],\"count_after\":%d,", RandomCount);
        Random(0); // range 0: returns 0 but still increments count
        printf("\"count_after_zero\":%d}", RandomCount);
        printf(",\n");
    }

    // 5b. Stateless SeededRandom used by C4Sky::Init for SkyDef list
    // selection. Pin zero-range and wrapping-u32 behavior directly from the
    // production inline in C4Random.h.
    arr_begin("rng_seeded_random");
    struct SeededRandomCase { uint32_t seed, range; };
    const SeededRandomCase seeded_random_cases[] = {
        {0u, 0u},
        {0u, 3u},
        {7u, 3u},
        {12345u, 100u},
        {0xffffffffu, 100u},
    };
    for (auto c : seeded_random_cases)
    {
        sep();
        printf("{\"seed\":%u,\"range\":%u,\"val\":%u}",
               c.seed, c.range, SeededRandom(c.seed, c.range));
    }
    arr_end();
    printf(",\n");

    // 6. Randomize3 / Rnd3: the 500-entry circular buffer (mass-mover, etc.).
    {
        const uint32_t seed = 9876;
        FixedRandom(seed);
        Randomize3();
        printf("\"rng_randomize3\":{\"seed\":%u,\"buffer\":[", seed);
        for (int i = 0; i < FRndRes; i++) { if (i) printf(","); printf("%d", FRndBuf3[i]); }
        printf("],\"rnd3_sequence\":[");
        for (int i = 0; i < 32; i++) { if (i) printf(","); printf("%d", Rnd3()); }
        printf("]}");
        printf(",\n");
    }

    // 7. Material corrosion execution RNG ordering (C4Material.cpp:701-711).
    //    Default corrosion short-circuits the second Random(100); user-defined
    //    corrosion performs one Random(100). Successful corrosion then consumes
    //    the smoke/sound effect Random() calls even in this side-effect-free
    //    oracle.
    arr_begin("material_corrode_rng");
    struct CorrodeCase { const char *name; uint32_t seed; bool custom; int corrosive, corrode, rate; };
    const CorrodeCase corrode_cases[] = {
        {"default_fail_first", 101, false, 0, 100, 0},
        {"default_success", 202, false, 100, 100, 0},
        {"default_maybe_resist", 303, false, 100, 35, 0},
        {"custom_success", 404, true, 0, 0, 100},
        {"custom_fail", 505, true, 0, 0, 0},
    };
    for (auto c : corrode_cases)
    {
        FixedRandom(c.seed);
        bool success;
        if (c.custom)
            success = Random(100) < c.rate;
        else
            success = (Random(100) < c.corrosive) && (Random(100) < c.corrode);
        if (success) consume_corrode_effect_rng();
        sep();
        printf("{\"name\":\"%s\",\"seed\":%u,\"custom\":%d,\"corrosive\":%d,\"corrode\":%d,\"rate\":%d,\"success\":%d,\"count\":%d,\"hold\":%u}",
               c.name, c.seed, c.custom ? 1 : 0, c.corrosive, c.corrode, c.rate,
               success ? 1 : 0, RandomCount, RandomHold);
    }
    arr_end();
    printf(",\n");

    // 8. Mass-mover transfer RNG ordering (C4MassMover.cpp:144,151).
    //    Every successful transfer consumes Random(10) for pixel-vs-material
    //    insertion before the Rnd3() immediate-execution decision.
    arr_begin("mass_mover_transfer_rng");
    struct TransferCase { const char *name; uint32_t seed; int iterations; };
    const TransferCase transfer_cases[] = {
        {"two_transfers_no_immediate", 2, 2},
        {"four_transfers_mixed_immediate", 9876, 4},
    };
    for (auto c : transfer_cases)
    {
        FixedRandom(c.seed);
        Randomize3();
        sep();
        printf("{\"name\":\"%s\",\"seed\":%u,\"iterations\":%d,\"calls\":[", c.name, c.seed, c.iterations);
        for (int i = 0; i < c.iterations; i++)
        {
            int random10 = Random(10);
            int rnd3 = Rnd3();
            if (i) printf(",");
            printf("{\"random10\":%d,\"rnd3\":%d,\"execute_immediately\":%d}",
                   random10, rnd3, !rnd3 ? 1 : 0);
        }
        printf("],\"count\":%d,\"hold\":%u}", RandomCount, RandomHold);
    }
    arr_end();
    printf(",\n");

    // 9. C4Value map-key hashing. `std::hash<C4Value>` seeds with the C4V_Type,
    //    then combines recursively with Boost's current hashCombine. C4ValueHash
    //    lookup correctness depends on this for nested array/map keys.
    printf("\"script_value_hash\":{\"sizeof_size_t\":%zu,\"hash_combine\":[", sizeof(std::size_t));
    printHashCombineCase("zero_zero", 0, 0);
    printf(",");
    printHashCombineCase("one_zero", 1, 0);
    printf(",");
    printHashCombineCase("zero_one", 0, 1);
    printf(",");
    printHashCombineCase("mixed", static_cast<std::size_t>(0x0123456789abcdefULL), static_cast<std::size_t>(0xfedcba9876543210ULL));
    printf("],\"values\":[");
    printHashValueCase("nil", std::hash<C4V_Type_Oracle>{}(C4V_Any_Oracle));
    printf(",");
    printHashValueCase("int_zero", hashInt(0));
    printf(",");
    printHashValueCase("int_42", hashInt(42));
    printf(",");
    printHashValueCase("int_minus_one", hashInt(-1));
    printf(",");
    printHashValueCase("bool_false", hashBool(false));
    printf(",");
    printHashValueCase("bool_true", hashBool(true));
    printf(",");
    printHashValueCase("id_CLNK", hashId("CLNK"));
    printf(",");
    printHashValueCase("id_1337", hashId("1337"));
    printf(",");
    printHashValueCase("string_empty", hashString(""));
    printf(",");
    printHashValueCase("string_alpha", hashString("alpha"));
    printf(",");
    printHashValueCase("string_16", hashString("abcdefghijklmnop"));
    printf(",");
    printHashValueCase("string_24", hashString("abcdefghijklmnopqrstuvwx"));
    printf(",");
    printHashValueCase("string_40", hashString("abcdefghijklmnopqrstuvwxyz0123456789ABCD"));
    printf(",");
    printHashValueCase("string_80", hashString("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    printf(",");
    printHashValueCase("array_1_true_x", hashArray({hashInt(1), hashBool(true), hashString("x")}));
    printf(",");
    const auto array23 = hashArray({hashInt(2), hashInt(3)});
    printHashValueCase("map_a1_b23", hashMap({{hashString("a"), hashInt(1)}, {hashString("b"), array23}}));
    printf(",");
    printHashValueCase("map_b23_a1", hashMap({{hashString("b"), array23}, {hashString("a"), hashInt(1)}}));
    printf(",");
    const auto mixedMap = std::initializer_list<std::pair<std::size_t, std::size_t>>{
        {hashInt(42), hashString("int")},
        {hashBool(true), hashInt(7)},
        {hashId("CLNK"), hashBool(false)},
        {hashObject(77), hashString("object")},
        {hashArray({hashInt(1), hashBool(true)}), hashId("1337")},
    };
    printHashValueCase("map_mixed_keys", hashMap(mixedMap));
    printf(",");
    printHashValueCase("map_mixed_keys_reversed", hashMap({
        {hashArray({hashInt(1), hashBool(true)}), hashId("1337")},
        {hashObject(77), hashString("object")},
        {hashId("CLNK"), hashBool(false)},
        {hashBool(true), hashInt(7)},
        {hashInt(42), hashString("int")},
    }));
    printf("]}");
    printf(",\n");

    // 9b. C4ScriptCnvMap type-conversion table (src/C4Value.cpp:488-598) +
    //     ConvertTo dispatch (src/C4Value.h:248-254). Locks BOTH the 81-cell
    //     table classification (a 9x9 grid of one-char codes, source row x
    //     destination column) and the per-(value, target, #strict) conversion
    //     result that drives getInt/getStr/... and parameter marshaling.
    printf("\"script_value_convert\":{\"type_count\":9,\"table\":[");
    for (int from = 0; from < 9; from++)
    {
        if (from) printf(",");
        printf("\"");
        for (int to = 0; to < 9; to++) printf("%c", cnvCode(C4ScriptCnvMapOracle[from][to]));
        printf("\"");
    }
    printf("],\"convert\":[");
    struct ConvCase { const char *name; int type; int32_t intData; };
    const ConvCase conv_cases[] = {
        {"nil",        C4V_Any_Oracle,    0},
        {"int_0",      C4V_Int_Oracle,    0},
        {"int_5000",   C4V_Int_Oracle,    5000},
        {"int_9999",   C4V_Int_Oracle,    9999},
        {"int_10000",  C4V_Int_Oracle,    10000},
        {"int_neg1",   C4V_Int_Oracle,    -1},
        {"bool_true",  C4V_Bool_Oracle,   1},
        {"bool_false", C4V_Bool_Oracle,   0},
        {"id_CLNK",    C4V_C4ID_Oracle,   0},
        {"string",     C4V_String_Oracle, 0},
        {"array",      C4V_Array_Oracle,  0},
        {"map",        C4V_Map_Oracle,    0},
    };
    bool firstConv = true;
    for (auto cc : conv_cases)
        for (int to = 0; to < 9; to++)
            for (int s = 1; s >= 0; s--)
            {
                if (!firstConv) printf(",");
                firstConv = false;
                bool strict = (s != 0);
                printf("{\"name\":\"%s\",\"from\":%d,\"to\":%d,\"strict\":%d,\"result\":%d}",
                       cc.name, cc.type, to, strict ? 1 : 0,
                       oracleConvertTo(cc.type, to, cc.intData, strict) ? 1 : 0);
            }
    printf("]}");
    printf(",\n");

    // 10. GetKiller/SetKiller host semantics. C4Script.cpp delegates these
    //     decisions to the production helper included above.
    printScriptKillerCases();
    printf(",\n");

    // 10b. FnEval's exact Obj -> Def -> Game.Script receiver selection and
    //      DirectExec's exact temporary Def/LocalNamed/parent setup.
    printEvalDirectExecContextCases();
    printf(",\n");

    // 10c. C4ID-only effect callbacks retain their affected object as the
    //      first callback argument but execute with a null object receiver.
    effect_position_oracle::printDefinitionCommandedEffectPositionCase();
    printf(",\n");

    // 10d. Effect callbacks alone are warning-only below STRICT3. The
    // production parameter conversion helper preserves the original object
    // on that warning path; strict integer and reference declarations reject
    // before the callback body can mutate or alias the carrier.
    effect_position_oracle::printEffectCallbackConversionCase();
    printf(",\n");

    // 11. C4Landscape::_PathFree coarse-cell occupancy. The edge-water case
    //     is the minimized Goldrush frame-143 PXS divergence.
    printLandscapePathCases();
    printf(",\n");

    // 12. DFA_WALK raw-xdir SetDir/TurnAction ordering. The input is the
    //     minimized Goldrush frame-170 WIPF divergence.
    printActionDirectionCase();
    printf(",\n");

    // 13. DFA_SWIM raw-xdir SetDir/TurnAction ordering. The input is the
    //     minimized Goldrush frame-219 FISH divergence.
    printSwimActionDirectionCase();
    printf(",\n");

    // 13b. Exact DFA_PUSH/PULL raw-xdir SetDir blocks and DFA_FIGHT's
    //      target-relative, equal-x-zero-call direction block.
    exec_action_direction_oracle::printCases();
    printf(",\n");

    // 14. C4Object::SetAction callback order/count. The first case is the
    //     minimized Goldrush frame-192 WIPF duplicate StartCall divergence.
    printActionCallbackCases();
    printf(",\n");

    // 14b. Exact DFA_CONNECT missing/incomplete-target branch: LineBreak(true)
    //      must precede AssignRemoval and its Destruction callback.
    printConnectMissingTargetCase();
    printf(",\n");

    // 14c. Exact later DFA_CONNECT geometry-break branch: LineBreak() has no
    //      argument, then follows the same AssignRemoval lifecycle.
    printConnectGeometryBreakCase();
    printf(",\n");

    // 15. C4SolidMask active graphics sampling. The variant_2 case is the
    //     minimized Goldrush frame-184 CTWR/SNKE contact divergence.
    printSolidMaskGraphicsCases();
    printf(",\n");

    // 15b. DefCore Scale -> Picture facet rect. Pins the percent->float
    //      conversion, C4Rect::Scaled's truncation, and Picture2Facet's
    //      phase-before-scale composition — the contract any HD (Scale != 100)
    //      content depends on.
    printDefPictureScaleCases();
    printf(",\n");

    // 16. HarpoonRace C4SGame conversion followed by authoritative
    //     C4GameParameters rule/goal placement, plus a source/count edge.
    printNetworkRuleGoalPlacementCases();
    printf(",\n");

    // 16b. Exact C4PlayerList linked count and Join capacity gate. The matrix
    //      pins zero-as-closed, one remaining slot, and exact-full rejection.
    player_join_capacity_oracle::printCases();
    printf(",\n");

    // 16c. The four MatchingLevel passes RestoreSavegameInfos runs when it
    //      associates joining players with a savegame's stored players.
    savegame_matching_oracle::printCases();
    printf(",\n");

    // 16c-2. The pass loop those levels run inside: which savegame player each
    //        joining player ends up associated with, and which associations
    //        C++ reports as "wild".
    savegame_matching_oracle::printAssociationCases();
    printf(",\n");

    // 16d. Component (C4IDList) order, which is inside the replay hash but had
    //      no comparable field: the ordered entries, GetNumberOfIDs counting a
    //      repeat twice, and GetIDCount resolving to the first match.
    component_order_oracle::printCases();
    printf(",\n");

    // 17. Exact DigOutMaterialCast spawn arguments and the twenty following
    //     Random draws on the same synced ledger.
    printDigOutMaterialCastCase();
    printf(",\n");

    // 18. Exact production ShakeObjects master-order/RNG gate sequence and
    //     C4Object::Fling raw fallback.
    printShakeObjectsCase();
    printf(",\n");
    blast_objects_oracle::printCases();
    printf(",\n");

    // 19. Exact C4Landscape::ClearPix / BlastFreePix / BlastFree scan,
    //     pre-count, IFT-preserving mutation, and RNG order.
    printBlastFreeCase();
    printf(",\n");

    // 20. Exact C4Landscape::ExecuteScan / DoScan conversion cadence and
    //     ScanX advancement across a wrapping two-column scan.
    printLandscapeScanCase();
    printf(",\n");

    // 21. Exact DFA_FLIGHT ContactAction arms, especially the low-speed
    //     `fDisabled` paths into FlatUp/Tumble instead of Hangle/Scale.
    printContactActionBottomFlightCases();
    printf(",\n");
    printContactActionTopSideFlightCases();
    printf(",\n");

    // 22. movement: per-frame sub-pixel accumulation (the Theme-C core).
    //    Mirrors C4Movement.cpp:260-261 (fix += dir) and :627 (ydir += gravity),
    //    WITHOUT landscape collision/contact (that is the per-pixel loop, item 4).
    arr_begin("movement");
    struct Scn { const char *name; int xdir_n, xdir_p, ydir_n, ydir_p, grav_n, grav_p, frames; };
    const Scn scns[] = {
        // SetXDir(15) => xdir=itofix(15,10)=1.5px/frame, gravity FIXED100(20)
        {"xdir15_grav20", 15, 10, 0, 1, 20, 100, 16},
        // sub-pixel gravity FIXED256(8) accumulating from rest
        {"grav256_8", 0, 1, 0, 1, 8, 256, 64},
        // mixed: xdir 4.6px (xdir=300 raw-ish via prec 100 -> 3.0), gravity FIXED100(10)
        {"mixed", 46, 10, -30, 10, 10, 100, 24},
    };
    for (auto s : scns)
    {
        C4Fixed fix_x = itofix(0), fix_y = itofix(0);
        C4Fixed xdir = itofix(s.xdir_n, s.xdir_p);
        C4Fixed ydir = itofix(s.ydir_n, s.ydir_p);
        C4Fixed grav = itofix(s.grav_n, s.grav_p);
        sep();
        printf("{\"name\":\"%s\",\"xdir\":%d,\"ydir0\":%d,\"grav\":%d,\"frames\":[",
               s.name, xdir.val, ydir.val, grav.val);
        for (int f = 0; f < s.frames; f++)
        {
            ydir += grav;     // C4Movement.cpp:627 (gravity)
            fix_x += xdir;    // C4Movement.cpp:260 (fix_x += xdir)
            fix_y += ydir;    // C4Movement.cpp (fix_y += ydir)
            if (f) printf(",");
            printf("{\"fix_x\":%d,\"fix_y\":%d,\"xdir\":%d,\"ydir\":%d,\"x\":%d,\"y\":%d}",
                   fix_x.val, fix_y.val, xdir.val, ydir.val, fixtoi(fix_x), fixtoi(fix_y));
        }
        printf("]}");
    }
    arr_end();
    printf(",\n");

    // 22b. pxs_execute: the per-tick PXS step (C4PXS.cpp:28-135), which
    //      `movement` above deliberately excludes and `pxs_allocation` does not
    //      reach.
    printPxsExecuteCases();
    printf(",\n");

    // 22c. insert_check: mrfInsertCheck, the landing arm pxs_execute
    //      deliberately excludes because it needs the reaction table.
    printInsertCheckCases();
    printf(",\n");

    // 22d. convert_check: mrfConvert's fallthrough and verdict rules.
    printConvertCases();
    printf(",\n");

    // 22e. insert_arm: mrfInsert's event gate and its once-only splash check.
    printInsertCases();
    printf(",\n");

    // 22e2. incinerate_arm: mrfIncinerate's asymmetric arms — which events can
    //       report unhandled, which one checks insertion before burning, and
    //       which one inserts a pixel that failed to ignite.
    printIncinerateCases();
    printf(",\n");

    // 22e3. poof_arm: mrfPoof's movement arm, where the unhandled outcome lives
    //       and where the insertion check gates the extraction and both draws.
    printPoofMoveCases();
    printf(",\n");

    // 22f. pxs_slots: New's positional slot reuse and Cast's forced draw order.
    printPxsSlotCases();
    printf(",\n");

    // 22g. pxs_load: Load's length arithmetic, chunk ceiling and per-chunk
    //      recount, plus the float conversion that only touches live slots.
    printPxsLoadCases();
    printf(",\n");

    // 23. DFA_FLOAT's raw C4Fixed bounds. C4DefCore's Physical member is
    // zero-initialized when [Physical] is absent (C4InfoCore.cpp:239-242),
    // and C4Object::ExecAction always clamps both directions to
    // FIXED100(Physical.Float), including that zero (C4Object.cpp:5291-5310).
    arr_begin("native_float");
    struct FloatCase { const char *name; int32_t xdir, ydir, physical_float; };
    const FloatCase float_cases[] = {
        {"zero_physical", 123456, -654321, 0},
        {"physical_100", 123456, -654321, 100},
    };
    for (auto s : float_cases)
    {
        C4Fixed xdir; xdir.val = s.xdir;
        C4Fixed ydir; ydir.val = s.ydir;
        const C4Fixed limit = FIXED100(s.physical_float);
        if (ydir < -limit) ydir = -limit; if (ydir > +limit) ydir = +limit;
        if (xdir > +limit) xdir = +limit; if (xdir < -limit) xdir = -limit;
        sep();
        printf("{\"name\":\"%s\",\"physical_float\":%d,\"xdir_before\":%d,\"ydir_before\":%d,\"xdir_after\":%d,\"ydir_after\":%d}",
               s.name, s.physical_float, s.xdir, s.ydir, xdir.val, ydir.val);
    }
    arr_end();
    printf("\n}\n");
    return 0;
}
