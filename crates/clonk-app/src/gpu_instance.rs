//! The process's `wgpu::Instance` registry.
//!
//! A `wgpu::Instance` is a `VkInstance`, and this process opens more than one
//! window: the console shell plus a window per `C4Viewport`. Building a
//! framebuffer per window used to build an instance per window too, so closing
//! a round destroyed a `VkInstance` while another window's swapchain was still
//! live.
//!
//! NVIDIA's Wayland WSI cannot survive that. It keeps the libwayland-client
//! entry points it `dlsym`s in a *process-global* dispatch table whose lifetime
//! is tied to a single `VkInstance`; `vkDestroyInstance` tears the table down,
//! and the next `vkAcquireNextImageKHR` from any surviving instance calls
//! through a nulled slot and faults with `rip=0` inside `libnvidia-glcore`
//! (clonk-org/clonk-rs#53 on driver 610.43.03, and the mirror-image teardown
//! crash in clonk-org/clonk-rs#54).
//!
//! So instances are created at most once per backend set and are *never*
//! dropped: the registry owns them for the life of the process. Handing out
//! clones is free — `wgpu::Instance` is a reference-counted handle — and one
//! instance for every window is what the rest of the ecosystem does anyway.

use pixels::wgpu;
use std::sync::{Mutex, OnceLock};

/// Look `backends` up, creating its entry once.
///
/// Generic over the instance type so the retention rule can be tested without
/// a GPU: what matters is that `create` runs once per backend set and that the
/// registry keeps its own handle afterwards.
fn retained_entry<T: Clone>(
    registry: &mut Vec<(wgpu::Backends, T)>,
    backends: wgpu::Backends,
    create: impl FnOnce() -> T,
) -> T {
    registry
        .iter()
        .find_map(|(known, entry)| (*known == backends).then(|| entry.clone()))
        .unwrap_or_else(|| {
            let entry = create();
            registry.push((backends, entry.clone()));
            entry
        })
}

fn registry() -> &'static Mutex<Vec<(wgpu::Backends, wgpu::Instance)>> {
    static REGISTRY: OnceLock<Mutex<Vec<(wgpu::Backends, wgpu::Instance)>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// The process's instance for `backends`, created on first use.
///
/// A backend set that turned out to have no usable adapter is retained too.
/// Enumeration is deterministic per backend set, so reusing that instance
/// fails and widens exactly as building a fresh one would, and retaining it is
/// the whole point: `vkDestroyInstance` is the call that must never run.
///
/// A poisoned registry cannot happen from the code below — nothing here can
/// panic while the lock is held — but recovering the guard rather than
/// unwrapping keeps a panic elsewhere from taking presentation down with it.
pub(crate) fn retained_instance(backends: wgpu::Backends) -> wgpu::Instance {
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    retained_entry(&mut registry, backends, || {
        pixels::create_instance(backends)
    })
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn one_instance_is_created_per_backend_set_however_many_windows_ask_for_it() {
        let mut registry = Vec::new();
        let mut creations = 0;
        let mut create = |registry: &mut Vec<_>, backends| {
            retained_entry(registry, backends, || {
                creations += 1;
                creations
            })
        };

        assert_eq!(create(&mut registry, wgpu::Backends::PRIMARY), 1);
        assert_eq!(create(&mut registry, wgpu::Backends::PRIMARY), 1);
        assert_eq!(create(&mut registry, wgpu::Backends::PRIMARY), 1);
        // A second window that had to widen its backends gets its own instance,
        // and the first set still resolves to the instance it already had.
        assert_eq!(create(&mut registry, wgpu::Backends::all()), 2);
        assert_eq!(create(&mut registry, wgpu::Backends::PRIMARY), 1);
    }

    // The crash this registry exists to prevent: closing a console viewport
    // dropped the last handle to that window's instance, and destroying a
    // `VkInstance` nulls NVIDIA's process-global Wayland dispatch table under
    // every window that is still presenting (clonk-org/clonk-rs#53).
    #[test]
    fn a_window_closing_never_drops_the_last_handle_to_an_instance() {
        let mut registry = Vec::new();
        let opened = retained_entry(&mut registry, wgpu::Backends::PRIMARY, || {
            std::sync::Arc::new(())
        });
        let instance = std::sync::Arc::downgrade(&opened);

        drop(opened);

        assert!(
            instance.upgrade().is_some(),
            "the registry must outlive every window that borrowed its instance"
        );
    }
}
