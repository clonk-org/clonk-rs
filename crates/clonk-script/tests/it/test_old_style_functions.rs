//! Pre-`func` C4Script function declarations used by the Tutorial butterfly.

use clonk_script::{Engine, Script, Value};

#[test]
fn old_style_labels_define_separate_callable_functions_below_strict_two() {
    // C4AulParse.cpp:1715-1763 accepts `Name:` declarations below #strict 2;
    // :1764-1805 ends one body at the next access modifier or label. The
    // tutorial `_BTF` script mixes qualified and unqualified labels exactly
    // this way (`private Fluttering:`, then `SitDown:` and `TakeOff:`).
    let script = Script::compile(
        r#"
        #strict

        protected Initialize:
          return(1);

        private Fluttering:
          return(2);

        SitDown:
          return(3);

        TakeOff:
          return(4);
        "#,
    )
    .expect("old-style functions compile");

    let mut engine = Engine::new();
    engine.add_script(script);
    for (name, value) in [
        ("Initialize", 1),
        ("Fluttering", 2),
        ("SitDown", 3),
        ("TakeOff", 4),
    ] {
        assert_eq!(
            engine.call(name, &[]).expect("old-style function runs"),
            Value::Int(value),
            "{name} must keep its own body"
        );
    }
}

#[test]
fn a_label_after_an_unclosed_function_ends_it_and_declares_nothing() {
    // C4AulParse.cpp:2216-2238: a bare old-style label in statement position
    // ends the function around it. The preparser has already shifted past the
    // label's name when it meets the colon, so that label declares nothing,
    // and the labels after it are declared as usual. InExantros' second act
    // had this shape until clonk-org/clonk-rs-content#81 closed its
    // `func Initialize() {`.
    let script = Script::compile(
        r#"
        #strict

        func Initialize() {
          return(1);

        RelaunchPlayer:
          return(2);

        NewNight:
          return(3);
        "#,
    )
    .expect("the unclosed function is recovered instead of aborting the script");

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine.call("Initialize", &[]).expect("Initialize runs"),
        Value::Int(1)
    );
    assert!(
        !engine.has_function("RelaunchPlayer"),
        "the label that ended Initialize declares nothing"
    );
    assert_eq!(
        engine.call("NewNight", &[]).expect("NewNight runs"),
        Value::Int(3)
    );
}

#[test]
fn strict_two_rejects_old_style_function_labels() {
    // C4AulParse.cpp:1715-1717 makes the legacy declaration a hard parse
    // error at #strict 2 and above. Script loading still recovers at the next
    // top-level declaration, so the error is retained as a diagnostic.
    let script = Script::compile("#strict 2\nLegacy:\n return(1);")
        .expect("the invalid declaration is quarantined instead of aborting the script");
    assert!(
        script
            .parse_diagnostics()
            .iter()
            .any(|error| error.message().contains("declaration")),
        "the strictness violation must remain observable"
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    engine
        .call("Legacy", &[])
        .expect_err("a rejected old-style declaration must not become callable");
}
