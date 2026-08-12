//! The component editor window's surface lifecycle.
//!
//! `C4ComponentHost::ShowDialog` is a Win32 modal dialog (`C4ComponentHost.cpp`)
//! and is not compiled on the reference build, so this window has no oracle to
//! follow beyond the commit rules the ported
//! [`clonk_engine::developer_components::ComponentHost`] already holds. It is
//! an ordinary software window like the toolbox and the object list, and it is
//! destroyed on close: a modal dialog has nothing to survive for.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::software_window::{build_software_window, SoftwarePresent, SoftwareWindow};
use crate::GameApp;
use winit::event_loop::ActiveEventLoop;

pub struct ComponentEditorWindowHost {
    pub(crate) surface: SoftwareWindow,
}

pub(crate) fn build_component_editor_window(
    target: &ActiveEventLoop,
    title: &str,
) -> anyhow::Result<ComponentEditorWindowHost> {
    use crate::developer_component_editor::{EDITOR_HEIGHT, EDITOR_WIDTH};

    Ok(ComponentEditorWindowHost {
        surface: build_software_window(
            target,
            title,
            true,
            None,
            EDITOR_WIDTH,
            EDITOR_HEIGHT,
            "developer component editor",
        )?,
    })
}

impl DeveloperWindowHost for ComponentEditorWindowHost {
    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(width, height);
    }

    fn request_redraw(&mut self) {
        self.surface.request_redraw();
    }

    fn set_visible(&mut self, visible: bool) {
        self.surface.set_visible(visible);
    }

    fn visible(&self) -> bool {
        self.surface.visible()
    }
}

impl DeveloperWindowPresenter<GameApp> for ComponentEditorWindowHost {
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        let (width, height) = self.surface.surface_extent();
        let surface = app.render_developer_component_editor(width, height);
        match self.surface.present_surface(surface.as_ref())? {
            SoftwarePresent::Presented | SoftwarePresent::Skipped => Ok(()),
            // A lost surface cancels the edit: `C4ComponentHost::Cancel`
            // mutates nothing, so the component is exactly as it was.
            SoftwarePresent::SurfaceLost => {
                Err("the developer component editor surface was lost".to_owned())
            }
        }
    }
}
