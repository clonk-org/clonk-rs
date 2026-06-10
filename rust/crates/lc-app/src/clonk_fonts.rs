//! Re-export of the CStdFont-faithful font builder (lives in lc-frontend so
//! dialog render tests can rasterize real fonts); integration tests against
//! the real Endeavour.ttf stay here.

pub use lc_frontend::clonk_fonts::build_font_set;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn endeavour_bytes() -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g/Endeavour.ttf");
        std::fs::read(path).expect("read Endeavour.ttf")
    }

    #[test]
    fn font_set_matches_cpp_line_heights() {
        let set = build_font_set(&endeavour_bytes()).expect("build font set");
        // (1303 - (-308)) * size / 1024 (StdFont.cpp:351).
        assert_eq!(set.title.line_height, 34);
        assert_eq!(set.caption.line_height, 25);
        assert_eq!(set.text.line_height, 22);
        assert_eq!(set.main_small.line_height, 20);
        assert_eq!(set.mini.line_height, 18);
        assert_eq!(set.title.cell_height, 35);
        assert_eq!(set.title.h_space, -1);
    }

    #[test]
    fn glyph_cells_have_shadowed_white_cores() {
        let set = build_font_set(&endeavour_bytes()).expect("build font set");
        let cell = set.title.glyph('A').expect("glyph A");
        assert!(cell.width > 5, "A should be wider than 5px");
        // The glyph core bakes to 254 (BltAlpha >>8 quirk) or 255 (pure src),
        // with full alpha; verify some near-white fully-opaque pixel exists.
        assert!(
            cell.pixels
                .iter()
                .any(|p| p.r >= 254 && p.g >= 254 && p.b >= 254 && p.a == 255),
            "expected an opaque white core pixel in 'A'"
        );
        // And the shadow: some dark, partially transparent pixel.
        assert!(
            cell.pixels
                .iter()
                .any(|p| p.r == 0 && p.a > 0 && p.a < 255),
            "expected a translucent black shadow pixel in 'A'"
        );
    }
}
