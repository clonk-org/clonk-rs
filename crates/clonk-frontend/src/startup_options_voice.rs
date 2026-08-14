//! Geometry for the port-only **Voice chat** group on the startup Options
//! Audio sheet.
//!
//! Deliberate divergence: LegacyClonk has no microphone and no voice settings,
//! so `C4StartupOptionsDlg`'s sound page (`C4StartupOptionsDlg.cpp:921-985`)
//! has no counterpart to this group. Proximity voice chat is a Rust-only
//! extension (see `clonk-app`'s `voice_chat` module) that until now could only
//! be configured through the generic Advanced Settings editor
//! (clonk-org/clonk-rs#452).
//!
//! The group is **purely additive**: C++'s own 2x5 grid clamps every cell to
//! `BookFont` line height x 5/2, so the three C++ groups occupy only the top
//! ~290 of the sheet's 462 logical pixels at 1280x720 and leave the rest blank.
//! This group is placed in exactly that slack, one `iIndentY1` below the Volume
//! group, which is why `frontend_group`, `game_group`, `volume_group` and every
//! child of theirs keep the pixel positions
//! `sound_layout_uses_cpp_grid_math_and_caption_font_client_inset` pins.
//!
//! Where the slack is smaller than one titled group box -- 640x480 leaves 50px
//! -- the group is omitted entirely rather than drawn over C++'s controls, and
//! the Advanced Settings editor remains the way to reach the `Voice` keys.

use crate::classic_gui::IntRect;
use crate::startup_options_dlg::Aligner;

/// Text extents the group measures, in the caller's fonts. The dialog owns the
/// fonts, so it measures and passes them in exactly as the Keyboard sheet's
/// `ControlSheetLayout::from_sheet` does.
#[derive(Clone, Copy, Debug)]
pub struct VoiceGroupMetrics {
    /// `BookFont` line height; one control row is this tall.
    pub row_height: i32,
    /// `GetStandardCheckBoxSize` width for the enable label: text + box + 4
    /// (`C4GuiCheckBox.cpp:151-162`).
    pub enable_check_width: i32,
    /// Measured width of the "Push to talk:" label.
    pub push_to_talk_label_width: i32,
    /// Measured width of the "Voice volume:" label.
    pub volume_label_width: i32,
    /// `GroupBox`'s stored client top margin: 4 + the GUI `CaptionFont` line
    /// height, the same SetTitle-before-SetFont quirk the C++ groups have
    /// (`C4Gui.h:993-1011`).
    pub title_line_height: i32,
}

/// Screen-coordinate geometry for the voice group and its five controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceGroupLayout {
    pub group: IntRect,
    /// Row 0, left: the microphone opt-in checkbox.
    pub enabled_check: IntRect,
    /// Row 0, right: "Push to talk:" and the key button it labels.
    pub push_to_talk_label: IntRect,
    pub push_to_talk_button: IntRect,
    /// Row 1: "Voice volume:" and the horizontal `ScrollBar` beside it.
    pub volume_label: IntRect,
    pub volume_slider: IntRect,
}

/// `C4GUI_ScrollBarHgt` (`C4Gui.h:44`) -- the same 16px bar the Volume group's
/// two sliders use.
const SLIDER_HEIGHT: i32 = 16;

/// The two rows this group lays out.
const ROWS: i32 = 2;

impl VoiceGroupLayout {
    /// The smallest group box that still holds both rows: the 4px frame on all
    /// sides, the stored caption inset on top, and `ROWS` rows at `row_height`
    /// separated by the inner `iIndentY2` margins.
    pub const fn minimum_height(metrics: &VoiceGroupMetrics, indent_y2: i32) -> i32 {
        Self::height_for_row_pitch(metrics, indent_y2, metrics.row_height)
    }

    /// The height that gives each row the Volume group's own pitch --
    /// `lh + 2*iIndentY2 + C4GUI_ScrollBarHgt` (`C4StartupOptionsDlg.cpp:967`)
    /// -- so the two groups read as one column where the sheet has the slack.
    pub const fn preferred_height(metrics: &VoiceGroupMetrics, indent_y2: i32) -> i32 {
        Self::height_for_row_pitch(
            metrics,
            indent_y2,
            metrics.row_height + indent_y2 * 2 + SLIDER_HEIGHT,
        )
    }

    const fn height_for_row_pitch(
        metrics: &VoiceGroupMetrics,
        indent_y2: i32,
        row_pitch: i32,
    ) -> i32 {
        8 + metrics.title_line_height + ROWS * (row_pitch + indent_y2) + indent_y2
    }

    /// Lays the group into the slack between `volume_group` and the bottom of
    /// `sheet`, or returns `None` when that slack cannot hold it.
    ///
    /// `indent_x1`/`indent_y1` are the dialog-wide `iIndentX1`/`iIndentY1` the
    /// sound page's own aligner uses, and `indent_y2` the halved margin its
    /// group children use (`C4StartupOptionsDlg.cpp:762-779, 921-985`).
    pub fn below(
        volume_group: IntRect,
        sheet: IntRect,
        indent_x1: i32,
        indent_y1: i32,
        indent_y2: i32,
        metrics: &VoiceGroupMetrics,
    ) -> Option<Self> {
        let top = volume_group.y + volume_group.h + indent_y1;
        let available = sheet.y + sheet.h - indent_y1 - top;
        if available < Self::minimum_height(metrics, indent_y2) {
            return None;
        }
        let height = Self::preferred_height(metrics, indent_y2).min(available);
        let group = IntRect {
            x: volume_group.x,
            y: top,
            w: volume_group.w,
            h: height,
        };
        let client = IntRect {
            x: group.x + 4,
            y: group.y + 4 + metrics.title_line_height,
            w: group.w - 8,
            h: group.h - 8 - metrics.title_line_height,
        };
        let rows = Aligner::new(
            IntRect {
                x: 0,
                y: 0,
                w: client.w,
                h: client.h,
            },
            indent_x1,
            indent_y2,
        );
        let child_abs = |rect: IntRect| IntRect {
            x: client.x + rect.x,
            y: client.y + rect.y,
            ..rect
        };
        let row = |index: i32| {
            child_abs(rows.get_grid_cell(0, 1, index, ROWS, -1, metrics.row_height, true, 1, 1))
        };

        // Row 0: the opt-in from the left, then the key button from the right
        // with its label immediately left of it.
        let mut top_row = Aligner::new(row(0), 1, 0);
        let enabled_check = top_row.get_from_left(metrics.enable_check_width, metrics.row_height);
        let button_width = top_row.inner_width() * 2 / 5;
        let push_to_talk_button = top_row.get_from_right(button_width, metrics.row_height);
        let push_to_talk_label =
            top_row.get_from_right(metrics.push_to_talk_label_width, metrics.row_height);

        // Row 1: the heading from the left, the bar filling what is left. The
        // C++ Volume group stacks its heading above its bar because it has a
        // whole grid row per slider; one row means an inline heading instead.
        let mut volume_row = Aligner::new(row(1), 1, 0);
        let volume_label = volume_row.get_from_left(metrics.volume_label_width, metrics.row_height);
        let volume_slider = volume_row.get_centered(volume_row.inner_width(), SLIDER_HEIGHT);

        Some(Self {
            group,
            enabled_check,
            push_to_talk_label,
            push_to_talk_button,
            volume_label,
            volume_slider,
        })
    }
}
