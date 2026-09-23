//! C4AulParseError prints `SGetLine` and `SLineGetCharacters` at the parser's
//! read position (C4AulParse.cpp:268-294; C4Strings.cpp:380-403), over the
//! buffer `LoadAppend` builds with a newline in front of the script
//! (C4ComponentHost.cpp:207). Every expected position below is one the pinned
//! oracle logged when it loaded these scripts, stored with CRLF line ends, as
//! definition scripts. The comments quote the oracle's messages.

/// The `LoadAppend` buffer of one script file made of `lines`.
fn load_append(lines: &[&str]) -> String {
    let mut buffer = String::from("\n");
    for line in lines {
        buffer.push_str(line);
        buffer.push_str("\r\n");
    }
    buffer
}

fn diagnostic_positions(lines: &[&str]) -> Vec<(usize, usize)> {
    let script = clonk_script::Script::compile_c4_string(&load_append(lines))
        .expect("parse errors are quarantined");
    let mut positions = script
        .parse_diagnostics()
        .iter()
        .map(|diagnostic| (diagnostic.line(), diagnostic.column()))
        .collect::<Vec<_>>();
    positions.sort_unstable();
    positions
}

#[test]
fn each_diagnostic_points_where_c4aul_prints_it() {
    let lines = [
        "#strict",
        "",
        "local counter;",
        "",
        "func ProbeParams(a, b)",
        "{",
        "  return a;",
        "}",
        "",
        "Garbage",
        "",
        "func AfterGarbage()",
        "{",
        "  return 1;",
        "}",
        "",
        "func ExpectedComma()",
        "{",
        "  ProbeParams(1, 2 345);",
        "}",
        "",
        "func WhileParams()",
        "{",
        "  while (ProbeParams(1), 0) Message(\"loop\");",
        "}",
        "",
        "protected func TypeParameter(id)",
        "{",
        "  return 0;",
        "}",
        "",
        "func MissingSemicolon()",
        "{",
        "  var value = 1",
        "  return value;",
        "}",
        "",
        "func BreakOutside()",
        "{",
        "  break;",
        "  return 1;",
        "}",
        "",
        "func UnknownEscape()",
        "{",
        r#"  return "abc\qdef";"#,
        "}",
        "",
        "func Navigation()",
        "{",
        "  return ProbeParams(1)~Other();",
        "}",
        "",
        "func UnexpectedKeyword()",
        "{",
        "  if (1) return 1; else else return 2;",
        "}",
        "",
        "func ExpectedParenthesis()",
        "{",
        "  return (1 + 2 Message);",
        "}",
        "",
        "global func UsesLocal()",
        "{",
        "  return counter;",
        "}",
        "",
        "func UnclosedString()",
        "{",
        "  return \"never closed;",
        "}",
    ];

    assert_eq!(
        diagnostic_positions(&lines),
        [
            (12, 5),  // declaration expected, but found identifier 'Garbage'
            (19, 23), // ',' or ')' expected, but found integer constant
            (24, 36), // while: passing 2 parameters, but only 1 are used
            (27, 33), // parameter has the same name as type id
            (35, 9),  // ',' or ';' expected, but found identifier
            (40, 9),  // 'break' is only allowed inside loops
            (46, 14), // unknown escape: q
            (51, 25), // unexpected prefix operator: ~
            (56, 29), // misplaced 'else'
            (61, 24), // ',' or ')' expected, but found identifier
            (66, 17), // using local variable in global function!
            (71, 24), // string not closed
        ]
    );
}

#[test]
fn a_nonstrict_inherited_call_points_past_its_parenthesis() {
    let lines = ["func LegacyInherited()", "{", "  return inherited(1);", "}"];

    // inherited disabled; use #strict syntax!
    assert_eq!(diagnostic_positions(&lines), [(3, 20)]);
}

#[test]
fn an_unresolved_inherited_call_points_past_its_parenthesis() {
    let lines = [
        "#strict",
        "",
        "func Orphan()",
        "{",
        "  return inherited();",
        "}",
    ];
    let mut engine = clonk_script::Engine::new();
    engine
        .load_script(&load_append(&lines))
        .expect("the script compiles");

    // inherited function not found, use _inherited to call failsafe
    let reported = engine
        .unresolved_inherited_diagnostics()
        .iter()
        .map(|diagnostic| (diagnostic.line, diagnostic.column))
        .collect::<Vec<_>>();
    assert_eq!(reported, [(5, 20)]);
}

#[test]
fn an_unexpected_character_points_past_it() {
    let lines = [
        "#strict 2",
        "",
        "func StrayCharacter()",
        "{",
        "  return 1 @ 2;",
        "}",
    ];

    // unexpected character '@' found
    assert_eq!(diagnostic_positions(&lines), [(5, 13)]);
}
