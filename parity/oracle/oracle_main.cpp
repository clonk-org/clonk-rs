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
#include <cstdio>
#include <functional>
#include <initializer_list>
#include <optional>
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
inline constexpr int32_t C4D_Living_Oracle = 1 << 3;
inline constexpr int32_t C4D_Object_Oracle = 1 << 4;
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
    } Action;
    struct ShapeState
    {
        int32_t AttachMat{MNone_Oracle};
        int32_t y{};
        int32_t Hgt{};
    } Shape;

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

int main()
{
    printf("{\n");

    // 1. itofix: whole-integer and precision-denominated construction.
    //    Covers gravity/velocity precision (default 10, FIXED100, FIXED256).
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
