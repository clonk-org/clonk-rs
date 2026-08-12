use std::{env, process::Command, process::Output};

const CHILD_MODE: &str = "LC_DEFAULT_INIT_TEST_CHILD";
const DEFAULT_MARKER: &str = "default init marker";
const APP_MARKER: &str = "application debug marker";
const DEPENDENCY_MARKER: &str = "dependency debug marker";
const INFO_MARKER: &str = "partial filter info marker";
const ERROR_MARKER: &str = "partial filter error marker";
const CONVERSION_MARKER: &str = "Unrecognized present mode 1000361000";
const INSTANCE_MARKER: &str = "vulkan instance warning marker";

fn run_child(test_name: &str, lc_log: Option<&str>, rust_log: Option<&str>) -> Output {
    let mut command = Command::new(env::current_exe().expect("integration-test executable"));
    command
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD_MODE, "1")
        .env_remove("LC_LOG")
        .env_remove("RUST_LOG")
        .envs(
            [("LC_LOG", lc_log), ("RUST_LOG", rust_log)]
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
    let output = command.output().expect("run the isolated logging child");
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn emit_filter_markers() {
    clonk_logging::init();
    tracing::info!("{INFO_MARKER}");
    tracing::error!("{ERROR_MARKER}");
}

#[test]
fn default_init_writes_to_stderr_without_escapes() {
    // `init()` serves the tool binaries, whose stdout is their deliverable and
    // is parsed by scripts. Diagnostics belong on stderr, and colour belongs
    // only on a terminal.
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::info!("{DEFAULT_MARKER}");
        return;
    }

    let output = run_child("default_init_writes_to_stderr_without_escapes", None, None);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(DEFAULT_MARKER),
        "log output landed on stdout: {stdout:?}"
    );
    assert!(
        stderr.contains(DEFAULT_MARKER),
        "log output missing from stderr"
    );
    assert!(
        !stderr.contains('\u{1b}'),
        "ANSI escapes written to a stderr that is not a terminal"
    );
}

#[test]
fn an_explicit_directive_keeps_the_dependency_suppression() {
    // Raising the level is what a user does to capture a bug report. If that
    // also unmuted the graphics stack, the report would be mostly wgpu.
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::debug!("{APP_MARKER}");
        tracing::debug!(target: "wgpu_core::device", "{DEPENDENCY_MARKER}");
        tracing::debug!(target: "naga::front", "{DEPENDENCY_MARKER}");
        return;
    }

    let output = run_child(
        "an_explicit_directive_keeps_the_dependency_suppression",
        None,
        Some("debug"),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(APP_MARKER),
        "the explicit directive still raises our own level"
    );
    assert!(
        !stderr.contains(DEPENDENCY_MARKER),
        "dependency debug output survived an explicit directive: {stderr}"
    );
}

#[test]
fn a_partially_invalid_directive_keeps_the_part_that_parses() {
    // Dropping the whole directive over one typo moves the level in whichever
    // direction the default happens to sit — here the user asked to go quieter
    // and would be made louder.
    if env::var_os(CHILD_MODE).is_some() {
        emit_filter_markers();
        return;
    }

    let output = run_child(
        "a_partially_invalid_directive_keeps_the_part_that_parses",
        Some("error,bogus=definitely-not-a-level"),
        None,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(ERROR_MARKER),
        "the directive that parses is still applied"
    );
    assert!(
        !stderr.contains(INFO_MARKER),
        "the requested level was not applied: {stderr}"
    );
}

#[test]
fn an_empty_directive_falls_back_to_the_default_level() {
    // Shell wrappers routinely export an unset variable as the empty string.
    if env::var_os(CHILD_MODE).is_some() {
        emit_filter_markers();
        return;
    }

    let output = run_child(
        "an_empty_directive_falls_back_to_the_default_level",
        Some(""),
        None,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(INFO_MARKER),
        "an empty directive silenced the default level: {stderr}"
    );
}

/// Current wgpu recognizes `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`, so the old
/// startup-noise workaround no longer has a legitimate warning to suppress.
/// Keep every future Vulkan conversion warning visible just like warnings from
/// the rest of the backend.
#[test]
fn current_wgpu_vulkan_warnings_reach_stderr() {
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::warn!(target: "wgpu_hal::vulkan::conv", "{CONVERSION_MARKER}");
        tracing::warn!(target: "wgpu_hal::vulkan::instance", "{INSTANCE_MARKER}");
        return;
    }

    let output = run_child("current_wgpu_vulkan_warnings_reach_stderr", None, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(CONVERSION_MARKER),
        "a Vulkan conversion warning was suppressed: {stderr}"
    );
    assert!(
        stderr.contains(INSTANCE_MARKER),
        "the rest of the Vulkan backend must keep warning: {stderr}"
    );
}

const CALLOOP_STALE_SOURCE_MARKER: &str =
    "[calloop] Received an event for non-existence source: TokenInner { id: 3, version: 4419, sub_id: 0 }";
const CALLOOP_PING_MARKER: &str = "[calloop] Failed to write a ping";

/// The Wayland key-repeat timer is reused rather than torn down, so this
/// warning should no longer fire in a real session. If it does, it is a
/// genuine calloop fault and must stay visible — same rule as the rest of
/// the windowing stack.
#[test]
fn calloop_loop_logic_warnings_reach_stderr() {
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::warn!(target: "calloop::loop_logic", "{CALLOOP_STALE_SOURCE_MARKER}");
        tracing::warn!(target: "calloop::sources::ping", "{CALLOOP_PING_MARKER}");
        return;
    }

    let output = run_child("calloop_loop_logic_warnings_reach_stderr", None, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(CALLOOP_STALE_SOURCE_MARKER),
        "a loop_logic warning was suppressed: {stderr}"
    );
    assert!(
        stderr.contains(CALLOOP_PING_MARKER),
        "the rest of calloop must keep warning: {stderr}"
    );
}
