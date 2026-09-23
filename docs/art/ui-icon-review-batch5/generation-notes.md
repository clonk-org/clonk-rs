# Hand gesture review, batch 5

The source is `planet/Graphics.c4g/Hand.png`, a seven-cell, 448×64 command-HUD sheet. Each source cell is 64×64. RedWolf Design authored the original content under [CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/), as documented in `planet/Graphics.c4g/COPYING`.

The seven 512×512 proposals are adaptations made with OpenAI's built-in image generation model. Each original pose controlled the corresponding output; the first generated hand was also supplied as a style reference for the others. The model baked a checkerboard into six outputs, so `prepare_images.py` extracts their skin-colored foreground and reconstructs transparent antialiased edges. It then fits each cutout to the original source cell's alpha bounds. `game-size/` previews show each proposed cell downsampled to 64×64. `support/` retains the unmodified generated outputs.

In review round 1, HAND-01 and HAND-05 through HAND-07 were approved. HAND-02, HAND-03, and HAND-04 were rejected because they did not look like proper hands. Their earlier previews and feedback are preserved in `history/`. The revised versions use the original poses and the approved HAND-05 style to make the grasp, flat hand, and pointing finger more anatomically clear. All seven were approved in round 2.

The app installs the approved 512×512 replacements for all seven cells while keeping the original `Hand.png` sheet dimensions and phase positions. The source sheet remains available as a fallback for custom game art.
