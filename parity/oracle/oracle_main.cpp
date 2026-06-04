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
//   * `Randomize3`/`Rnd3` are reproduced verbatim from `src/C4Random.cpp`
//     (10 trivial lines around the real `Random()`); kept in sync via the
//     provenance comment below.
//
// Regenerate the golden with `parity/oracle/gen_golden.sh`.

#include <cstdint>
#include <cstdio>

#include "oracle_fixed.h" // generated from src/Fixed.h by gen_golden.sh
#include <C4Random.h>     // real engine header (no DEBUGREC)

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

    // 7. movement: per-frame sub-pixel accumulation (the Theme-C core).
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
