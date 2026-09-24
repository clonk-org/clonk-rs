# Startup dialog button and vertical scrollbar review

Serve the repository root with `python3 -m http.server 9185` and open
`http://127.0.0.1:9185/docs/art/startup-dialog-controls/`. Drag the Back or
Player Selection comparisons, then drag the Wipf in either live scrollbar.

The before/after crops on the page come from matching 1280 × 720 game frames:
`clonk-app --dump-menu-frame <path> --menu-view options`, `scenarios`, and
`plrsel`. The Back crop is at `(96,632)` with size `170 × 48`; the scenario
scrollbar crop is at `(548,183)` with size `38 × 374`; the Player Selection
button strip is at `(28,653)` with size `1220 × 52`. The live scrollbar draws
the same cells from `StartupBookScroll.png` and `StartupBookScrollHD.png` that
the game uses, with the classic or HD Wipf at the same logical position.

The normal profile reuses the approved main-menu HD wood planks on startup
dialog buttons and the approved HD book-scroll atlas on vertical scrollbars.
The classic profile retains its original facets for presentation comparisons.
