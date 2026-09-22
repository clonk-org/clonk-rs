# High-resolution startup icons

The options tabs use `crates/clonk-app/assets/StartupOptionIconsHD.png`, a
1536×256 RGBA strip containing six 256×256 cells: Program, Graphics, Audio,
Keyboard, Gamepad, and Network. Each cell is still drawn in its original
32×32 logical rectangle. The GPU keeps the larger source texture when the UI
is scaled; the software renderer samples it with bilinear filtering.

`crates/clonk-app/assets/StartupWipfHD.png` is a 256×256 RGBA replacement for
the full-body Wipf scrollbar thumb. It retains the original 16×16 logical
bounds and travel range. The arrows, track, and colored player-color thumbs
remain the original artwork.

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

Visual review: [before and after textures](startup-icon-comparison.png) and
[the rendered Audio options screen](startup-icons-in-game.png). The comparison
enlarges the original pixels without smoothing and displays both sets at the
same size; the in-game capture uses the Normal profile at 1280×720.
