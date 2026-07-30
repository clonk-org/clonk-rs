//! `C4Viewport::Draw`'s phase order, and where the console overlay sits in it.
//!
//! The console's edit cursor draws inside the ordinary viewport pass, not on
//! top of a finished frame, so *where* its hook goes is a parity question
//! (`C4Viewport.cpp:1023-1119`). Three details are easy to get wrong:
//!
//! - The hook is **after** the foreground/custom-GUI objects and **before**
//!   `DrawOverlay`, which is the per-player HUD. Putting it last would draw the
//!   edit cursor over the HUD; putting it with the world objects would let the
//!   HUD cover it.
//! - It runs **after the border inset is undone**. C++ restores the full `cgo`
//!   rect and the unclipped primary clipper *before* the hook
//!   (`:1093-1099`), so the console draws across the whole viewport, including
//!   the border strips the world was clipped out of.
//! - It is gated on `!Application.isFullScreen` (`:1106`) — a fullscreen game
//!   never draws it, whatever the console state.
//!
//! Fog of war is disabled before both (`:1090`), so neither the console overlay
//! nor the HUD is modulated by it.

/// One phase of `C4Viewport::Draw`, in call order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ViewportDrawPhase {
    /// Landscape border tiles. Overlay passes only, so a full-map screenshot is
    /// not framed (`:1031-1035`).
    Borders,
    /// The border inset and primary clipper are applied (`:1038-1040`).
    ClipToBorders,
    /// `ClrModMap` setup for a fog-of-war player (`:1045-1053`).
    FogOfWarBegin,
    Sky,
    BackObjects,
    Landscape,
    /// Drawn unclipped, as C++ notes.
    Pxs,
    Objects,
    GlobalParticles,
    /// `ForeObjects.DrawIfCategory(..., C4D_Parallax, true)` — the inverted
    /// pass, i.e. everything that is *not* parallax (`:1082`).
    ForegroundParallax,
    /// Only with `ShowPathfinder` (`:1085`).
    PathFinder,
    /// `DrawCursors`, skipped only when a film *and* a replay (`:1088`).
    Cursors,
    FogOfWarEnd,
    /// The border inset and clipper are undone (`:1093-1099`).
    RestoreFullRect,
    /// `ForeObjects.DrawIfCategory(..., C4D_Parallax, false)` — "custom GUI
    /// objects" (`:1102`).
    CustomGuiObjects,
    /// `Console.EditCursor.Draw(cgo)`, windowed builds only (`:1106`).
    ConsoleOverlay,
    /// `DrawOverlay(cgo)` — the per-player HUD (`:1108`).
    PlayerHud,
    /// `Game.Network.DrawStatus`, only with `ShowNetstatus` (`:1111-1112`).
    NetworkStatus,
    /// `NoPrimaryClipper` (`:1118`).
    ReleaseClipper,
}

/// What the pass knows about its context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportDrawContext {
    /// `fDrawOverlay` — false for a full-map screenshot, which then draws no
    /// borders, no HUD and no clipper.
    pub draw_overlay: bool,
    /// `Application.isFullScreen`. The console overlay never draws here.
    pub fullscreen: bool,
    /// `pPlr && pPlr->fFogOfWar`.
    pub fog_of_war: bool,
    /// `Game.GraphicsSystem.ShowPathfinder`.
    pub show_pathfinder: bool,
    /// `Game.GraphicsSystem.ShowNetstatus`.
    pub show_netstatus: bool,
    /// `Game.C4S.Head.Film && Game.C4S.Head.Replay` — cursors are skipped only
    /// when **both** hold.
    pub film_replay: bool,
}

/// The phases `C4Viewport::Draw` actually runs, in order.
pub fn viewport_draw_phases(context: ViewportDrawContext) -> Vec<ViewportDrawPhase> {
    use ViewportDrawPhase::*;
    let mut phases = Vec::new();
    if context.draw_overlay {
        phases.push(Borders);
        phases.push(ClipToBorders);
    }
    if context.fog_of_war {
        phases.push(FogOfWarBegin);
    }
    phases.extend([Sky, BackObjects, Landscape, Pxs, Objects, GlobalParticles]);
    phases.push(ForegroundParallax);
    if context.show_pathfinder {
        phases.push(PathFinder);
    }
    // `if (!Film || !Replay)` — skipped only when both are set.
    if !context.film_replay {
        phases.push(Cursors);
    }
    // Unconditional: C++ disables the map whether or not it enabled one.
    phases.push(FogOfWarEnd);
    if context.draw_overlay {
        phases.push(RestoreFullRect);
    }
    phases.push(CustomGuiObjects);
    if !context.fullscreen {
        phases.push(ConsoleOverlay);
    }
    if context.draw_overlay {
        phases.push(PlayerHud);
    }
    if context.show_netstatus {
        phases.push(NetworkStatus);
    }
    if context.draw_overlay {
        phases.push(ReleaseClipper);
    }
    phases
}

#[cfg(test)]
mod tests {
    use super::ViewportDrawPhase::*;
    use super::*;

    fn context() -> ViewportDrawContext {
        ViewportDrawContext {
            draw_overlay: true,
            fullscreen: false,
            fog_of_war: false,
            show_pathfinder: false,
            show_netstatus: false,
            film_replay: false,
        }
    }

    fn index(phases: &[ViewportDrawPhase], phase: ViewportDrawPhase) -> Option<usize> {
        phases.iter().position(|held| *held == phase)
    }

    // C4Viewport.cpp:1023-1119 — the console hook's exact place in the pass.
    #[test]
    fn detached_viewport_overlay_hook_precedes_player_hud() {
        let phases = viewport_draw_phases(context());

        let console = index(&phases, ConsoleOverlay).expect("windowed builds draw it");
        let hud = index(&phases, PlayerHud).expect("an overlay pass draws the HUD");
        let custom_gui = index(&phases, CustomGuiObjects).expect("custom GUI objects");
        let objects = index(&phases, Objects).expect("world objects");

        assert!(
            objects < custom_gui && custom_gui < console,
            "the console hook comes after the world and the custom GUI objects"
        );
        assert!(
            console < hud,
            "the console hook must precede the per-player HUD, or the HUD covers it"
        );

        // The border inset is undone *before* the hook, so the console draws
        // across the full viewport rect rather than the world's clipped one.
        let restore = index(&phases, RestoreFullRect).expect("the rect is restored");
        assert!(restore < console);
        assert!(
            index(&phases, ClipToBorders).expect("clipped") < index(&phases, Sky).expect("sky"),
            "the world is drawn inside the border inset"
        );

        // Fog of war is off before both, so neither is modulated by it.
        assert!(index(&phases, FogOfWarEnd).expect("fow off") < console);

        // A fullscreen game never draws the console overlay, whatever else is
        // enabled — but everything else is unchanged.
        let fullscreen = viewport_draw_phases(ViewportDrawContext {
            fullscreen: true,
            ..context()
        });
        assert_eq!(index(&fullscreen, ConsoleOverlay), None);
        assert!(index(&fullscreen, PlayerHud).is_some());

        // A screenshot pass (`fDrawOverlay == false`) drops the borders, the
        // clipper and the HUD — but still draws the console overlay, because
        // that gate is `isFullScreen`, not `fDrawOverlay`.
        let screenshot = viewport_draw_phases(ViewportDrawContext {
            draw_overlay: false,
            ..context()
        });
        for absent in [
            Borders,
            ClipToBorders,
            RestoreFullRect,
            PlayerHud,
            ReleaseClipper,
        ] {
            assert_eq!(
                index(&screenshot, absent),
                None,
                "{absent:?} is overlay-only"
            );
        }
        assert!(index(&screenshot, ConsoleOverlay).is_some());

        // Cursors are skipped only when film *and* replay both hold.
        for (film_replay, expected) in [(false, true), (true, false)] {
            let phases = viewport_draw_phases(ViewportDrawContext {
                film_replay,
                ..context()
            });
            assert_eq!(index(&phases, Cursors).is_some(), expected);
        }

        // The optional diagnostics sit where C++ puts them.
        let diagnostics = viewport_draw_phases(ViewportDrawContext {
            show_pathfinder: true,
            show_netstatus: true,
            fog_of_war: true,
            ..context()
        });
        let pathfinder = index(&diagnostics, PathFinder).expect("pathfinder");
        assert!(
            index(&diagnostics, ForegroundParallax).expect("foreground") < pathfinder
                && pathfinder < index(&diagnostics, Cursors).expect("cursors")
        );
        assert!(
            index(&diagnostics, PlayerHud).expect("hud")
                < index(&diagnostics, NetworkStatus).expect("netstatus")
        );
        assert!(
            index(&diagnostics, FogOfWarBegin).expect("fow on")
                < index(&diagnostics, Sky).expect("sky")
        );
    }
}
