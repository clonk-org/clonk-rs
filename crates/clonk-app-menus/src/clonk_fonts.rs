//! Re-export of the CStdFont-faithful font builder (lives in clonk-frontend so
//! dialog render tests can rasterize real fonts).

pub use clonk_frontend::clonk_fonts::{build_font_set, build_tooltip_font};
