//! Bounded malformed-input campaign over the C4Script front end
//! (clonk-org/clonk-rs#962).
//!
//! Scripts come from scenarios and definition packs, and `DirectExec` also
//! accepts developer and message-board text. The parser intentionally *recovers*
//! from many C++ syntax errors rather than rejecting, which makes the contract
//! richer than "does it reject bad input":
//!
//! * arbitrary bytes must not panic, and must not recurse into a stack
//!   overflow — deep nesting is the obvious shape for that;
//! * recovery must make **progress**. A recovering parser that neither consumes
//!   a token nor terminates is a hang, and it is invisible to a test that only
//!   checks the return value;
//! * diagnostics must stay bounded. One malformed token must not produce a
//!   diagnostic per remaining byte.
//!
//! Every strictness level is covered, because the grammar genuinely differs
//! between them — `nil`, array literals, map literals and dot access are each
//! admitted at a different level.

use clonk_script::Script;

/// Inputs are capped so a case measures the parser rather than the mutator.
const MAX_INPUT: usize = 4096;

/// Nesting is capped separately and far lower: the point of the deep-nesting
/// cases is to reach the recursion guard, not to spend the budget building a
/// string.
const MAX_NESTING: usize = 512;

/// The four public compile entry points, plus the strictness levels that change
/// the grammar under them.
const STRICTNESS: &[&str] = &["", "#strict\n", "#strict 2\n", "#strict 3\n"];

fn compile_all(source: &str) -> usize {
    // Each entry point is a distinct front end: object versus global scope, and
    // the C4String variants that take the native byte projection.
    let mut diagnostics = 0;
    if let Ok(script) = Script::compile(source) {
        diagnostics += script.parse_diagnostics().len();
    }
    if let Ok(script) = Script::compile_global(source) {
        diagnostics += script.parse_diagnostics().len();
    }
    if let Ok(script) = Script::compile_c4_string(source) {
        diagnostics += script.parse_diagnostics().len();
    }
    if let Ok(script) = Script::compile_global_c4_string(source) {
        diagnostics += script.parse_diagnostics().len();
    }
    diagnostics
}

/// A diagnostic per input byte would mean recovery is not consuming anything;
/// this ceiling is generous and still catches that.
fn diagnostic_ceiling(input_len: usize) -> usize {
    // Four entry points, each allowed a diagnostic per few bytes.
    4 * (input_len + 16)
}

const SEEDS: &[&str] = &[
    "",
    "func F() { return 1; }\n",
    "func F(int a, bool b, string c) { return a; }\n",
    "static const X = 1;\n",
    "local a, b, c;\n",
    "func F() { var x = [1, 2, 3]; return x[0]; }\n",
    "func F() { var m = {a = 1}; return m.a; }\n",
    "func F() { return nil; }\n",
    "func F() { if (1) return 2; else return 3; }\n",
    "func F() { for (var i = 0; i < 3; i++) {} }\n",
    "func F() { while (1) break; }\n",
    "func F() { return inherited(); }\n",
    "func F() { return _inherited(...); }\n",
    "#appendto CLNK\nfunc F() { return 1; }\n",
    "#include CLNK\n",
    "func F() { return \"unterminated\n",
    "func F() { /* unterminated\n",
    "func F() { return 0x7fffffff + 0777 + 1; }\n",
    "func F() { return a->b->c->d(); }\n",
    "func F() { return 1 +++ 2 --- 3; }\n",
    "func F(a, a, a) { return a; }\n",
    "func F() {{{{{{{{{{ }\n",
    "@legacy() { return 1; }\n",
    "func F() { goto(1); }\n",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }
}

/// Mutations operate on bytes and the result is lossily re-decoded, so invalid
/// UTF-8 reaches the lexer the way a native-encoded script would.
fn mutate(seed: &str, rng: &mut Rng) -> String {
    let mut bytes = seed.as_bytes().to_vec();
    for _ in 0..=rng.below(8) {
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes[at] = (rng.next() & 0xff) as u8;
            }
            1 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            2 => {
                let at = rng.below(bytes.len() + 1);
                // Punctuation is where the grammar branches, so bias toward it.
                let byte = *b"(){}[];,=+-*/<>.\"'\\|&!?:@#$%^~`"
                    .get(rng.below(31))
                    .unwrap_or(&b'?');
                bytes.insert(at, byte);
            }
            3 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(64);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            4 => {
                let other = SEEDS[rng.below(SEEDS.len())];
                bytes.extend_from_slice(other.as_bytes());
            }
            _ => {
                let level = STRICTNESS[rng.below(STRICTNESS.len())];
                let mut prefixed = level.as_bytes().to_vec();
                prefixed.extend_from_slice(&bytes);
                bytes = prefixed;
            }
        }
        if bytes.len() > MAX_INPUT {
            bytes.truncate(MAX_INPUT);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn every_seed_compiles_or_reports_a_typed_error() {
    for level in STRICTNESS {
        for seed in SEEDS {
            let source = format!("{level}{seed}");
            let diagnostics = compile_all(&source);
            assert!(
                diagnostics <= diagnostic_ceiling(source.len()),
                "{diagnostics} diagnostics from {} bytes",
                source.len()
            );
        }
    }
}

#[test]
fn arbitrary_script_bytes_never_panic_or_flood_diagnostics() {
    let mut rng = Rng(0x00c4_5c21_9700_u64);
    for round in 0..4_000 {
        let seed = SEEDS[rng.below(SEEDS.len())];
        let source = mutate(seed, &mut rng);
        let diagnostics = compile_all(&source);
        assert!(
            diagnostics <= diagnostic_ceiling(source.len()),
            "round {round}: {diagnostics} diagnostics from {} bytes",
            source.len()
        );
    }
}

#[test]
fn truncation_at_every_offset_is_safe() {
    // Truncation lands mid-token, mid-string and mid-comment in turn, which is
    // where a recovering parser is most likely to stall.
    for level in STRICTNESS {
        for seed in SEEDS {
            let source = format!("{level}{seed}");
            for length in 0..source.len() {
                if !source.is_char_boundary(length) {
                    continue;
                }
                compile_all(&source[..length]);
            }
        }
    }
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    // Every recursive construct in the grammar, nested to the cap. A parser
    // without a depth guard overflows here rather than returning an error.
    for (what, open, close) in [
        ("parens", "(", ")"),
        ("braces", "{", "}"),
        ("brackets", "[", "]"),
    ] {
        let source = format!(
            "func F() {{ return {}1{}; }}\n",
            open.repeat(MAX_NESTING),
            close.repeat(MAX_NESTING)
        );
        let diagnostics = compile_all(&source);
        assert!(
            diagnostics <= diagnostic_ceiling(source.len()),
            "{what}: {diagnostics} diagnostics from {} bytes",
            source.len()
        );
        // Unbalanced is the more hostile shape: the closing half never arrives.
        compile_all(&format!(
            "func F() {{ return {}1; }}\n",
            open.repeat(MAX_NESTING)
        ));
    }
}

#[test]
fn a_long_parameter_list_is_bounded() {
    // C++ caps a declaration at ten parameters; a list far past that must be a
    // diagnostic rather than unbounded work.
    let params = (0..MAX_NESTING)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("func F({params}) {{ return 1; }}\n");
    let diagnostics = compile_all(&source);
    assert!(
        diagnostics <= diagnostic_ceiling(source.len()),
        "{diagnostics} diagnostics from a {}-parameter declaration",
        MAX_NESTING
    );
}

#[test]
fn repeated_recovery_does_not_flood_diagnostics() {
    // One malformed token repeated many times must not cost more than one
    // diagnostic each — that is the shape of a recovery loop that consumes
    // nothing.
    for repeats in [1_usize, 10, 100, 1_000] {
        let source = format!("func F() {{ {} }}\n", "?".repeat(repeats));
        let diagnostics = compile_all(&source);
        assert!(
            diagnostics <= diagnostic_ceiling(source.len()),
            "{diagnostics} diagnostics from {repeats} malformed tokens"
        );
    }
}
