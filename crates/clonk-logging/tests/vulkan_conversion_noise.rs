use std::{env, process::Command};

const CHILD_MODE: &str = "LC_VULKAN_CONV_CHILD";
const CONVERSION_MARKER: &str = "Unrecognized present mode 1000361000";
const INSTANCE_MARKER: &str = "vulkan instance warning marker";

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

    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args([
            "--exact",
            "current_wgpu_vulkan_warnings_reach_stderr",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .env_remove("LC_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("run the isolated logging child");

    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
