# In-game menu icon review — batch 3

All nine source cells are 35×35 pixels. `MENU-01` through `MENU-05` come from
`planet/Graphics.c4g/Menu.png`; `MENU-06` through `MENU-09` come from
`planet/Graphics.c4g/Options.png`. Exact cell coordinates, source hashes,
preview hashes, and completed review feedback are in `manifest.json`.
The original game artwork is attributed to RedWolf Design under CC BY-NC 4.0
in `planet/Graphics.c4g/COPYING`.

Round 1 approved MENU-01 Save game, MENU-05 Display, MENU-07 Music, and
MENU-08 FPS display. Their image files remain byte-for-byte unchanged. The five
original previews that received revision requests are retained under
`history/`; the user's exact feedback is in `history/review-round-1.json`.

The revisions were generated with OpenAI's built-in imagegen edit mode using
the original cells and the previous previews as references:

| ID | Revision for round 2 |
| --- | --- |
| MENU-02 Goals | Restore the brown background panel, tall gray stone tower on the right, lower stone structure, and gold. Approved HUD building and gold art provided style references. |
| MENU-03 Rules | Treat the red element as a flag attached to a full visible pole, retaining the gold plaque and blue/gray shapes. |
| MENU-04 Hostility | Flatten the crossed swords to broad pale-gray Clonk-style blades with plain guards and short brown grips. |
| MENU-06 Settings | Arrange large gold and small bronze gears so a tooth visibly engages the opposite gear's valley. |
| MENU-09 Audio | Render the symbol as a golden speaker cone, using the existing `StartupOptionIconsHD.png` speaker as a reference, not an ear. |

Round 2 approved eight icons and left MENU-03 Rules pending. Its exact feedback
is in `history/review-round-2.json`, and that preview is retained as
`history/menu-rules-round-2.png`. The user clarified that the source icon
contains a **blue flag, red lightning bolt, and gray hammer** on a gold square
with chamfered corners. The round 3 Rules preview was regenerated with the
original 35×35 cell as the layout authority, the first preview as a painting
style reference, and the approved HUD construction hammer as a tool-form
reference. The prompt required all four clipped plaque corners, a blue flag
with a left pole, an angular red bolt, and a blunt gray hammer head on a brown
diagonal handle. A second built-in imagegen pass extracted true alpha from
the generated checkerboard backdrop. The other eight previews are unchanged.

For round 3, the user accepted the blue flag, red bolt, and gray hammer but
clarified that the high-resolution plaque must **not** have chamfered edges.
The exact feedback is in `history/review-round-3.json`, and the clipped-corner
preview is retained as `history/menu-rules-round-3.png`. The new Rules preview
edits only the plaque to four square 90-degree outer corners, preserves the
three approved foreground symbols, and has a true alpha background. The other
eight previews remain byte-for-byte unchanged.

The user approved all nine previews in round 4, including the square-cornered
Rules plaque. The final decision is preserved in `history/review-round-4.json`.
`prepare_runtime.sh` removes the ivory backdrop from Hostility and Settings,
fits all nine approved previews to their original occupied bounds, and writes
280×280 transparent runtime sprites. Options.png has enabled Music, FPS, and
Audio phases, so the script also overlays a red mark derived from the approved
`ui-icons/checkbox-on.png` onto those three approved base icons. The sheet
installer replaces matching stock cells only, retaining the 35×35 logical
layout and any custom cell art. Other Options cells remain candidates for a
later batch.
