use clonk_logging::resolve_filter_directive;

fn directive(lc_log: Option<&str>, rust_log: Option<&str>, default_level: &str) -> String {
    resolve_filter_directive(lc_log, rust_log, default_level).0
}

fn rejected(lc_log: Option<&str>, rust_log: Option<&str>, default_level: &str) -> Vec<String> {
    resolve_filter_directive(lc_log, rust_log, default_level).1
}

#[test]
fn filter_directives_override_verbose_default_in_priority_order() {
    assert_eq!(
        directive(Some("clonk_app=trace"), Some("warn"), "debug"),
        "clonk_app=trace"
    );
    assert_eq!(directive(None, Some("warn"), "debug"), "warn");
}

#[test]
fn filter_selection_uses_default_for_missing_or_unusable_directives() {
    assert_eq!(directive(None, None, "debug"), "debug");
    assert_eq!(
        directive(
            Some("clonk_app=definitely-not-a-level"),
            Some("trace"),
            "info"
        ),
        "info",
        "a directive with nothing usable falls back rather than silencing everything"
    );
    assert_eq!(
        directive(Some("   "), None, "info"),
        "info",
        "a whitespace-only directive counts as unset"
    );
}

#[test]
fn a_partially_valid_directive_keeps_its_usable_parts() {
    assert_eq!(
        directive(Some("error,bogus=definitely-not-a-level"), None, "info"),
        "error"
    );
    assert_eq!(
        rejected(Some("error,bogus=definitely-not-a-level"), None, "info"),
        vec!["bogus=definitely-not-a-level".to_string()],
        "the dropped part is reported so the user can correct it"
    );
    assert!(
        rejected(Some("error"), None, "info").is_empty(),
        "a fully valid directive reports nothing"
    );
}
