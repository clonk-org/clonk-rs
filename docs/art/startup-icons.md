# High-resolution startup icons

The options tab icons use `crates/clonk-app/assets/StartupOptionIconsHD.png`, a
1536×256 RGBA strip containing six 256×256 cells: Program, Graphics, Audio,
Keyboard, Gamepad, and Network. The options layout determines the logical icon size (up to 32×32). The GPU
keeps the larger source texture when the UI is scaled; the software renderer
samples it with bilinear filtering.

The paper and metal tab backing uses
`crates/clonk-app/assets/StartupTabClipHD.png`, a 960×640 RGBA adaptation of
the original 120×80 `StartupTabClip.png`. The high-resolution backing retains
the original tab geometry and loop opening, and is drawn at the same logical
120×80 size. [Its source, preparation, and native GPU comparison](startup-tab/README.md)
are kept separately from the icons.

`crates/clonk-app/assets/StartupWipfHD.png` is a 256×256 RGBA replacement for
the full-body Wipf scrollbar thumb. It retains the original 16×16 logical
bounds and travel range. The options sliders also use the 384×384
`StartupBookScrollHD.png` atlas for their arrows and repeating track. Its
[source artwork and native GPU comparison](startup-slider/README.md) are kept
separately. The colored player-color thumbs remain the original artwork.

These assets are embedded in the application and enabled by default in the
Normal compatibility profile. The LegacyClonk profile uses the original
artwork. The legacy graphics groups remain unchanged, preserving the
presentation oracle and classic fallback inputs. The replacement Wipf is
shared by the options sliders, player-list scrollbar, and scenario information
scrollbar.

The source artwork is by RedWolf Design and is licensed under
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/). See
`planet/Graphics.c4g/COPYING` and `crates/clonk-app/assets/COPYING`.
These are OpenAI imagegen adaptations made for Clonk Rust on 2026-09-22.

References were `StartupOptionIcons.png` and the user's options screenshots.
The paint can was corrected to have a round cylindrical body and a separate
wire handle. The Wipf uses the complete seated creature from
`content/Objects.c4d/Animals.c4d/Wipf.c4d/Graphics.png` and the exact slider
thumb at `(16,16,16,16)` in `StartupBookScroll.png`; it is not the separate
Wipf portrait.

The approved generated images included opaque preview backgrounds. Runtime
preparation removed the connected beige/gray background, discarded isolated
matte fragments, and contracted the matte by one source pixel before a
Lanczos reduction. The resulting images have real alpha, including partial
coverage on their edges. Each silhouette is fitted proportionally into the
original cell's occupied bounds. The Wipf's preview slider track is removed;
the game draws its own track behind the transparent sprite.

The selected generation prompts and source-image hashes are recorded in
[startup-icon-prompts.json](startup-icon-prompts.json). They describe the
approved artwork before runtime matting and atlas packing.

Visual review: [before and after textures](startup-icon-comparison.png),
[native GPU before/after details](startup-icons-gpu-comparison.png), and
[the rendered Audio options screen](startup-icons-in-game.png), plus the
[tab backing comparison](startup-tab/in-game-comparison.png). The texture
comparison enlarges original pixels without smoothing and displays both sets
at the same size. The in-game images come from the live retained GPU renderer
at 3840×2160 with 3× display scaling, including native fonts. The detail view
crops those frames at native pixel size. Both frames use identical settings;
only the replacement artwork is removed for the before frame.

The software `--dump-menu-frame` path renders at logical resolution. Enlarging
its 1280×720 output cannot demonstrate the GPU's high-resolution artwork.
Regenerate the native GPU frames on a machine with a supported GPU using:

```sh
CLONK_HD_OPTIONS_CAPTURE=/tmp/options-audio-gpu.png cargo nextest run -p clonk-app \
  -E 'test(scaled_options_gpu_frame_keeps_all_nine_high_resolution_sources)'
```

This writes the updated frame to the requested path and the original-art frame
to `/tmp/options-audio-gpu.before.png`. Without the environment variable, the
test verifies that all nine high-resolution sources reach the GPU command
stream without requiring a GPU or producing files.
