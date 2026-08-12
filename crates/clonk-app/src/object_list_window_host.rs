//! The object list window's surface lifecycle.
//!
//! `C4ObjectListDlg::Open` creates its own top level — resizable, a utility
//! type hint, role `"objectlist"`, titled `"Objects"`, 180×300, transient for
//! the console (`C4ObjectListDlg.cpp:726-740`) — and, unlike the toolbox,
//! **destroys** it on close: the `"destroy"` handler nulls the window and the
//! model rather than hiding them (`:592-597`). `DeveloperWindows::close`
//! already treats `HostPurpose::ObjectList` that way.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::software_window::{build_software_window, SoftwarePresent, SoftwareWindow};
use crate::GameApp;
use winit::event_loop::ActiveEventLoop;

pub struct ObjectListWindowHost {
    pub(crate) surface: SoftwareWindow,
}

/// Create the object list window and its framebuffer.
///
/// The four hints beyond resizability have no winit equivalent, exactly as
/// they have none for the toolbox.
pub(crate) fn build_object_list_window(
    target: &ActiveEventLoop,
) -> anyhow::Result<ObjectListWindowHost> {
    use crate::developer_object_list_view::{
        OBJECT_LIST_HEIGHT, OBJECT_LIST_TITLE, OBJECT_LIST_WIDTH,
    };

    Ok(ObjectListWindowHost {
        surface: build_software_window(
            target,
            OBJECT_LIST_TITLE,
            true,
            None,
            OBJECT_LIST_WIDTH,
            OBJECT_LIST_HEIGHT,
            "developer object list",
        )?,
    })
}

impl ObjectListWindowHost {
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        self.surface.surface_extent()
    }
}

impl DeveloperWindowHost for ObjectListWindowHost {
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

impl DeveloperWindowPresenter<GameApp> for ObjectListWindowHost {
    /// Draw the live object tree.
    ///
    /// C++ keeps a `GtkTreeStore` incrementally in step through
    /// `OnObjectAdded`/`OnObjectRemove` (`:486-590`). The port rebuilds the
    /// rows from the snapshot every redraw instead: the snapshot *is* the
    /// object list, there is no separate model to drift from it, and the
    /// window redraws only on the console's own graphics pass.
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        let (width, height) = self.surface.surface_extent();
        let surface = app.render_developer_object_list(width, height);
        match self.surface.present_surface(Some(&surface))? {
            SoftwarePresent::Presented | SoftwarePresent::Skipped => Ok(()),
            // The list holds nothing the console needs, so a lost surface
            // closes it exactly as its own close button would.
            SoftwarePresent::SurfaceLost => {
                Err("the developer object list surface was lost".to_owned())
            }
        }
    }
}
