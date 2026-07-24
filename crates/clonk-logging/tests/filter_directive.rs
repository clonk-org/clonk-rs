use clonk_logging::select_filter_directive;

#[test]
fn filter_directives_override_verbose_default_in_priority_order() {
    assert_eq!(
        select_filter_directive(Some("clonk_app=trace"), Some("warn"), "debug"),
        "clonk_app=trace"
    );
    assert_eq!(select_filter_directive(None, Some("warn"), "debug"), "warn");
}

#[test]
fn filter_selection_uses_default_for_missing_or_invalid_directive() {
    assert_eq!(select_filter_directive(None, None, "debug"), "debug");
    assert_eq!(
        select_filter_directive(
            Some("clonk_app=definitely-not-a-level"),
            Some("trace"),
            "info",
        ),
        "info"
    );
}
