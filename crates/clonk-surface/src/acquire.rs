//! Bounded drawable acquisition.
//!
//! Acquiring a drawable can fail transiently, and the obvious loop — retry
//! until it succeeds — is a trap: a surface that stays outdated holds the
//! event-loop callback forever, so the resize that would fix it is never
//! processed. Every transient status therefore gets at most one reconfigure
//! before the frame is abandoned, and the caller is told whether a frame was
//! actually presented.

/// What asking the surface for a drawable produced.
///
/// Generic over the frame so the policy can be exercised without a GPU; the
/// wgpu mapping lives in [`crate::window`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acquisition<F> {
    /// A drawable, ready to render into.
    Success(F),
    /// A drawable that no longer matches the surface, but is still usable.
    Suboptimal(F),
    /// The surface configuration is stale; nothing was acquired.
    Outdated,
    /// The surface is gone and must be recreated by its owner.
    Lost,
    /// The compositor is not showing this window.
    Occluded,
    /// The drawable did not arrive in time.
    Timeout,
    /// The driver rejected the request.
    Validation,
}

/// Why acquisition failed in a way the caller has to act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AcquireError {
    /// The surface must be rebuilt: wgpu requires recreation, not merely
    /// reconfiguration, so this is reported rather than retried.
    #[error("the window surface was lost")]
    SurfaceLost,
    /// The driver rejected the acquisition.
    #[error("the window surface failed validation")]
    Validation,
}

/// What to do about a transient status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Reconfigure,
    UseFrame,
    SkipFrame,
}

/// The transient statuses, which differ only in what a *second* occurrence means.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transient {
    Suboptimal,
    Outdated,
}

/// Reconfigure the first time; after that a suboptimal frame is still a frame,
/// but an outdated one never became usable and is dropped.
const fn action(status: Transient, reconfigured: bool) -> Action {
    match (status, reconfigured) {
        (_, false) => Action::Reconfigure,
        (Transient::Suboptimal, true) => Action::UseFrame,
        (Transient::Outdated, true) => Action::SkipFrame,
    }
}

/// Acquire a drawable, reconfiguring at most once.
///
/// `Ok(None)` means no frame is available this tick and nothing was drawn —
/// the caller must not report a presentation.
pub fn acquire_drawable<F>(
    mut acquire: impl FnMut() -> Acquisition<F>,
    mut reconfigure: impl FnMut(),
) -> Result<Option<F>, AcquireError> {
    let mut reconfigured = false;
    loop {
        let next = match acquire() {
            Acquisition::Success(frame) => return Ok(Some(frame)),
            Acquisition::Suboptimal(frame) => (Transient::Suboptimal, Some(frame)),
            Acquisition::Outdated => (Transient::Outdated, None),
            Acquisition::Lost => return Err(AcquireError::SurfaceLost),
            Acquisition::Validation => return Err(AcquireError::Validation),
            Acquisition::Occluded | Acquisition::Timeout => return Ok(None),
        };
        match (action(next.0, reconfigured), next.1) {
            (Action::Reconfigure, frame) => {
                // The acquired frame has to go before the surface is
                // reconfigured; wgpu rejects a configure while a drawable from
                // that surface is still alive (parasyte/pixels#450).
                drop(frame);
                reconfigure();
                reconfigured = true;
            }
            (Action::UseFrame, Some(frame)) => return Ok(Some(frame)),
            (Action::UseFrame, None) | (Action::SkipFrame, _) => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // A drawable that reports itself outdated is reconfigured once. If it is
    // still outdated the frame is skipped rather than retried again: the
    // unbounded retry this replaces trapped the event-loop callback, so the
    // resize that would have fixed the surface could never be processed.
    #[test]
    fn an_outdated_drawable_reconfigures_once_and_then_skips_the_frame() {
        let acquisitions = Cell::new(0);
        let reconfigurations = Cell::new(0);

        let frame = acquire_drawable::<u32>(
            || {
                acquisitions.set(acquisitions.get() + 1);
                Acquisition::Outdated
            },
            || reconfigurations.set(reconfigurations.get() + 1),
        );

        assert_eq!(frame, Ok(None));
        assert_eq!(acquisitions.get(), 2);
        assert_eq!(reconfigurations.get(), 1);
    }

    // A suboptimal drawable is still a drawable. Reconfiguring may fix it, but
    // if it does not, presenting a slightly mismatched frame beats presenting
    // nothing — so unlike an outdated surface this ends in a frame, not a skip.
    #[test]
    fn a_persistently_suboptimal_drawable_is_used_after_one_reconfiguration() {
        let acquisitions = Cell::new(0);
        let reconfigurations = Cell::new(0);

        let frame = acquire_drawable(
            || {
                acquisitions.set(acquisitions.get() + 1);
                Acquisition::Suboptimal(7_u32)
            },
            || reconfigurations.set(reconfigurations.get() + 1),
        );

        assert_eq!(frame, Ok(Some(7)));
        assert_eq!(acquisitions.get(), 2);
        assert_eq!(reconfigurations.get(), 1);
    }

    // Reconfiguration is what usually clears a transient status, so the frame
    // acquired on the second attempt is the one that gets presented.
    #[test]
    fn a_drawable_that_recovers_after_reconfiguration_is_presented() {
        let acquisitions = Cell::new(0);

        let frame = acquire_drawable(
            || {
                acquisitions.set(acquisitions.get() + 1);
                if acquisitions.get() == 1 {
                    Acquisition::Outdated
                } else {
                    Acquisition::Success(42_u32)
                }
            },
            || {},
        );

        assert_eq!(frame, Ok(Some(42)));
        assert_eq!(acquisitions.get(), 2);
    }

    // A lost surface cannot be repaired by reconfiguring it — wgpu requires the
    // surface to be recreated — so this reports out immediately instead of
    // spending a reconfigure that cannot help.
    #[test]
    fn a_lost_surface_is_reported_without_reconfiguring() {
        let acquisitions = Cell::new(0);
        let reconfigurations = Cell::new(0);

        let frame = acquire_drawable::<u32>(
            || {
                acquisitions.set(acquisitions.get() + 1);
                Acquisition::Lost
            },
            || reconfigurations.set(reconfigurations.get() + 1),
        );

        assert_eq!(frame, Err(AcquireError::SurfaceLost));
        assert_eq!(acquisitions.get(), 1);
        assert_eq!(reconfigurations.get(), 0);
    }

    // An occluded or timed-out surface is a normal quiet tick, not a fault.
    // It yields no frame, which is what stops the caller reporting a
    // presentation that never reached the compositor.
    #[test]
    fn an_occluded_or_timed_out_surface_yields_no_frame_and_no_error() {
        let reconfigurations = Cell::new(0);
        let bump = || reconfigurations.set(reconfigurations.get() + 1);

        assert_eq!(
            acquire_drawable::<u32>(|| Acquisition::Occluded, bump),
            Ok(None)
        );
        assert_eq!(
            acquire_drawable::<u32>(|| Acquisition::Timeout, bump),
            Ok(None)
        );
        assert_eq!(reconfigurations.get(), 0);
    }

    // Validation is a driver rejection, not a transient status: retrying it
    // would just fail the same way.
    #[test]
    fn a_validation_failure_is_reported_without_reconfiguring() {
        let reconfigurations = Cell::new(0);

        let frame = acquire_drawable::<u32>(
            || Acquisition::Validation,
            || reconfigurations.set(reconfigurations.get() + 1),
        );

        assert_eq!(frame, Err(AcquireError::Validation));
        assert_eq!(reconfigurations.get(), 0);
    }

    // The whole point of the bound: however long the surface misbehaves, this
    // returns. An unbounded retry here is what trapped the event loop.
    #[test]
    fn acquisition_always_terminates_however_the_surface_misbehaves() {
        for status in [
            Acquisition::Suboptimal(1_u32),
            Acquisition::Outdated,
            Acquisition::Occluded,
            Acquisition::Timeout,
        ] {
            let acquisitions = Cell::new(0);
            let _ = acquire_drawable(
                || {
                    acquisitions.set(acquisitions.get() + 1);
                    assert!(
                        acquisitions.get() <= 2,
                        "acquisition retried more than once for {status:?}"
                    );
                    status
                },
                || {},
            );
        }
    }
}
