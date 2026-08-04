//! `planet/System.c4g/MenuRangeRow.c` and `MarsOrderCapsule.c` **replace**
//! shipped ClonkMars functions rather than extending them, because what they
//! change sits in the middle of those functions: which rows a page emits, what
//! `CreateMenu` is given, which symbol an unchosen enum row gets, and whether
//! an order is priced before it is bought. An `#appendto` override always
//! wins, so if the content submodule ever moves and ClonkMars changes one of
//! those functions, the new version is silently discarded and nothing fails.
//!
//! These tests are that alarm. They read the shipped source out of the content
//! submodule — `Menu.c4d` is a packed C4Group, so this goes through
//! `clonk_resources::Group` rather than the filesystem — and pin the exact
//! text of every function the overrides replace or lean on. A content bump
//! that touches one fails here with the new source in the message, so the
//! override can be re-checked against it deliberately.
//!
//! A failure is not a defect. It means: read the new shipped function, decide
//! whether the divergence in `PORT_STATUS.md` still says what we want, and
//! re-pin.

use crate::support::real_scenario::content_root;
use clonk_resources::Group;

/// FNV-1a. `DefaultHasher` is explicitly not stable across Rust releases, so
/// it cannot back a checked-in constant.
fn digest(source: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in source.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Line endings and trailing blanks are not semantics: the content checkout
/// may be CRLF or LF depending on the platform's autocrlf.
fn normalize(source: &str) -> String {
    source
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn group_source(relative_group: &str, file: &str) -> String {
    let path = content_root().join(relative_group);
    let group =
        Group::open(&path).unwrap_or_else(|error| panic!("{} opens: {error}", path.display()));
    let bytes = group
        .read_file(file)
        .unwrap_or_else(|error| panic!("{}/{file} reads: {error}", path.display()));
    normalize(&clonk_script::c4_string_from_bytes(&bytes))
}

/// The body of one C4Script function, from its `func` keyword to the brace
/// that closes it.
fn function_source(source: &str, name: &str) -> String {
    let needle = format!("func {name}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("`{name}` is still declared"));
    // Include the access modifier that precedes `func`, since dropping one is
    // exactly the kind of change worth noticing.
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let mut depth = 0_usize;
    let mut seen_brace = false;
    for (offset, character) in source[start..].char_indices() {
        match character {
            '{' => {
                depth += 1;
                seen_brace = true;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let body = &source[line_start..start + offset + 1];
                    // A digest over an empty or truncated slice would pin
                    // nothing while still passing.
                    assert!(
                        body.len() > 40 && body.contains('{'),
                        "`{name}` extracted as {body:?}, which is not a function body"
                    );
                    return body.to_string();
                }
            }
            _ => {}
        }
        if seen_brace && depth == 0 {
            break;
        }
    }
    panic!("`{name}` has a balanced body")
}

const MENU2_RUNTIME: &str = "ClonkMars.c4d/Helpers.c4d/Menu2.c4d/Menu.c4d";
const MENU2_LIBRARY: &str = "ClonkMars.c4d/Helpers.c4d/Menu2.c4d/System.c4g";
const MARS_BASE: &str = "ClonkMars.c4d/Structures.c4d/Base.c4d";

#[test]
fn the_menu2_functions_the_override_replaces_are_unchanged() {
    // Replaced wholesale by planet/System.c4g/MenuRangeRow.c. If ClonkMars
    // changes one of these, our copy silently discards the change.
    for (group, function, expected) in [
        (MENU2_RUNTIME, "ShowMenu", 0xf43e_e8bf_0970_fe23_u64),
        (MENU2_RUNTIME, "ShowEnum", 0x9919_d70c_bfa9_1e9e),
        (MENU2_RUNTIME, "ShowRange", 0xa66f_71b2_703a_c407),
        (MENU2_RUNTIME, "MenuQueryCancel", 0xb3e6_0846_182b_656d),
    ] {
        let source = function_source(&group_source(group, "Script.c"), function);
        assert_eq!(
            digest(&source),
            expected,
            "ClonkMars' `{function}` changed, and planet/System.c4g/MenuRangeRow.c \
             replaces it — the new version is being discarded. Re-check the override \
             against this and re-pin:\n{source}"
        );
    }
}

#[test]
fn the_menu2_functions_the_override_calls_are_unchanged() {
    // Called, not replaced: the override leans on their exact behaviour.
    for (group, file, function, expected) in [
        (
            MENU2_RUNTIME,
            "Script.c",
            "IncreaseRange",
            0x14c1_cdb0_3819_8e3e_u64,
        ),
        (
            MENU2_RUNTIME,
            "Script.c",
            "DecreaseRange",
            0x4138_dd54_8ee1_d564,
        ),
        (MENU2_RUNTIME, "Script.c", "Finished", 0x2205_a737_a6e1_fc5f),
        (
            MENU2_RUNTIME,
            "Script.c",
            "CreateDummy",
            0x5bfa_ed91_a3a3_54aa,
        ),
        (
            MENU2_LIBRARY,
            "Menu.c",
            "GetMenuValues",
            0xf0a8_8bd3_ac11_51bf,
        ),
    ] {
        let source = function_source(&group_source(group, file), function);
        assert_eq!(
            digest(&source),
            expected,
            "ClonkMars' `{function}` changed and planet/System.c4g/MenuRangeRow.c \
             calls it. Re-check the override against this and re-pin:\n{source}"
        );
    }
}

#[test]
fn the_base_functions_the_override_replaces_or_calls_are_unchanged() {
    for (function, expected) in [
        ("OrderCapsule", 0xd216_867a_72f8_e448_u64), // replaced
        ("CapsuleCheck", 0xb242_4e0b_d1a7_5cab),     // called
        ("CreateCapsule", 0x8cbb_b458_ecc8_59ca),    // called
        ("ContainedUp", 0x91a1_911a_a3f6_3766),      // builds the template we render
    ] {
        let source = function_source(&group_source(MARS_BASE, "Script.c"), function);
        assert_eq!(
            digest(&source),
            expected,
            "ClonkMars' `{function}` changed and \
             planet/System.c4g/MarsOrderCapsule.c depends on it. Re-check the \
             override against this and re-pin:\n{source}"
        );
    }
}

#[test]
fn the_menu2_template_indices_the_override_hardcodes_are_unchanged() {
    // MenuRangeRow.c spells the MS4C_* array indices as literals, because
    // Menu2's own System.c4g is not registered when planet/System.c4g parses.
    // Those literals are only correct while these declarations are.
    let source = group_source(MENU2_LIBRARY, "Menu.c");
    let collapsed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    for declaration in [
        "static const MS4C_Typ_Bool = 0;",
        "static const MS4C_Typ_Enum = 1;",
        "static const MS4C_Typ_Range = 2;",
        "static const MS4C_Typ_Submenu = 3;",
        "static const MS4C_Symbol_Index = 0;",
        "static const MS4C_Caption_Index = 1;",
        "static const MS4C_Hash_Index = 2;",
        "static const MS4C_Sequence_Index = 3;",
        "static const MS4C_Type_Index = 0;",
        "static const MS4C_Cond_Index = 1;",
        "static const MS4C_Name_Index = 2;",
        "static const MS4C_Id_Index = 3;",
        "static const MS4C_Data_Index = 4;",
    ] {
        assert!(
            collapsed.contains(declaration),
            "`{declaration}` no longer holds, and MenuRangeRow.c hardcodes that index"
        );
    }
    // AddRangeChoice's payload order is what `data[0..3]` means.
    assert!(
        collapsed.contains("MenuPut(menu,aPath,Key,[MS4C_Typ_Range,aCond,szName,idItem,[iMin,iMax,iStep,iDefault]]);"),
        "the range payload is no longer [min, max, step, current]"
    );
}
