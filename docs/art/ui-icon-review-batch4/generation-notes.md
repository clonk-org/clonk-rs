# Gamepad icon review — batch 4

`planet/Graphics.c4g/Gamepad.png` is a 320×36 sheet with four 80×36 controller
phases. The startup Options device selector uses these phases, and the sheet
also reaches the HUD graphics resources. The original art is credited to
RedWolf Design under CC BY-NC 4.0 in `planet/Graphics.c4g/COPYING`.

The four source cells are in `before/`. The first cell, enlarged only for model
reference, and the previously approved controller in
`crates/clonk-app/assets/StartupOptionIconsHD.png` informed one OpenAI built-in
imagegen reconstruction. The prompt required the original white/charcoal
silhouette, dark-blue D-pad, purple/green/red/yellow button layout, and no
player number. It used the approved artwork only for material and lighting
style. The resulting image is preserved in `support/controller-generated-opaque.png`.

The model rendered a checkerboard into the pixels despite a transparency
request. `prepare_review.py` removes that backdrop by selecting the
controller's connected dark outline, filling its interior, and using it as
the alpha mask. It then fits the controller into a 1280×576 transparent canvas.

Round 1 used a Times Bold Italic face for the red player numbers. The user
requested more faithful numbers. Those four previews are retained under
`history/`, and the exact feedback is in `history/review-round-1.json`. Round 2
extracts each red numeral's own connected silhouette from its original
80×36 source cell, smooths the pixel steps, and restores a red fill with a
dark edge and shadow at the original position. The shared controller base is
byte-for-byte unchanged; only the digits differ. The user found those numbers
scuffed, so round 2 previews are retained in `history/` with that feedback in
`history/review-round-2.json`.

Round 3 uses four OpenAI built-in imagegen numeral edits preserved as
`support/digit-*-generated-opaque.png`. `prepare_review.py` extracts only each
red numeral, including its bright highlights and dark outline; it leaves the
rest of each model image out. The extracted digits are scaled and placed using
the red numeral bounds in the original 80×36 cells. The shared controller base
has the same SHA-256 (`824ebf3107bf92998fea397397b010be7a8ce48d6a30abf8ae5ec294de87ad54`)
as round 2. The `game-size/` files show each revised sprite downscaled to its
original display dimensions.

The user liked round 3 but asked for a better 3. Round 4 replaces only
`support/digit-3-generated-opaque.png` and its derived preview with a flatter
top, clearer waist, and more even lower curve. The previous PAD-03 is in
`history/gamepad-3-round-3.png`; the feedback is in
`history/review-round-3.json`. PAD-01, PAD-02, and PAD-04 have the exact same
file hashes as round 3, and the shared controller base is unchanged. The user
approved the final PAD-03 in round 4; `history/review-round-4.json` records
that approval.

`prepare_runtime.py` reduces each approved 1280×576 preview to a 640×288
runtime sprite, eight times its original 80×36 cell. The app installs these as
region replacements in the stock `Gamepad.png` sheet, so the Options selector,
player properties, and in-game HUD keep their original layout geometry while
drawing the full-resolution art. A scenario-provided custom cell stays intact.

The source-cell coordinates, dimensions, hashes, and decisions are in
`manifest.json`. The adaptations retain the original CC BY-NC 4.0 license and
attribution in `crates/clonk-app/assets/COPYING`.
