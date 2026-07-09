// LegacyClonk C++<->Rust differential parity oracle.
//
// This program is the GOLDEN ORACLE generator for the Phase-1 differential
// harness. It exercises the determinism-critical C++ primitives that the Rust
// port (rust/crates/lc-engine) must reproduce bit-for-bit, and emits the
// results as JSON to stdout. The Rust side (lc-engine/tests/parity_differential.rs)
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
//   * `C4LandscapePath.h` is the production coarse-cell traversal used by
//     `C4Landscape::_PathFree`; the oracle feeds it real PixCnt-style inputs.
//   * `Randomize3`/`Rnd3` are reproduced verbatim from `src/C4Random.cpp`
//     (10 trivial lines around the real `Random()`); kept in sync via the
//     provenance comment below.
//
// Regenerate the golden with `parity/oracle/gen_golden.sh`.

#include <cstdint>
#include <cstdio>
#include <functional>
#include <initializer_list>
#include <string_view>
#include <utility>

#include "oracle_fixed.h" // generated from src/Fixed.h by gen_golden.sh
#include <C4Random.h>     // real engine header (no DEBUGREC)
#include <C4LandscapePath.h> // real production coarse-path traversal
#include <C4ScriptKiller.h> // real production script-host helper

extern long SineTable[9001]; // defined by the generated sine_table.cpp

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

    // 11. C4Landscape::_PathFree coarse-cell occupancy. The edge-water case
    //     is the minimized Goldrush frame-143 PXS divergence.
    printLandscapePathCases();
    printf(",\n");

    // 12. movement: per-frame sub-pixel accumulation (the Theme-C core).
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
    printf("\n}\n");
    return 0;
}
