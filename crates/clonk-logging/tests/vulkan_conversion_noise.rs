use std::{env, process::Command};

const CHILD_MODE: &str = "LC_VULKAN_CONV_CHILD";
const CONVERSION_MARKER: &str = "Unrecognized present mode 1000361000";
const INSTANCE_MARKER: &str = "vulkan instance warning marker";

/// `wgpu_hal::vulkan::conv` warns once per surface configuration for every
/// `VkPresentModeKHR` the pinned wgpu does not know — a driver advertising
/// `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT` (`1000361000`) puts four of them on
/// stderr before the main menu appears. The enumeration is *supposed* to skip
/// modes wgpu cannot map, so the warning reports a driver newer than the
/// pinned wgpu, not a fault, and it cannot be fixed at the call site: wgpu is
/// pinned by `pixels`.
///
/// The rest of `wgpu_hal` keeps its warnings, which do report faults.
#[test]
fn unmappable_vulkan_enum_warnings_stay_off_stderr() {
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::warn!(target: "wgpu_hal::vulkan::conv", "{CONVERSION_MARKER}");
        tracing::warn!(target: "wgpu_hal::vulkan::instance", "{INSTANCE_MARKER}");
        return;
    }

    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args([
            "--exact",
            "unmappable_vulkan_enum_warnings_stay_off_stderr",
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
        !stderr.contains(CONVERSION_MARKER),
        "an unmappable-enum warning reached stderr: {stderr}"
    );
    assert!(
        stderr.contains(INSTANCE_MARKER),
        "the rest of the Vulkan backend must keep warning: {stderr}"
    );
}
