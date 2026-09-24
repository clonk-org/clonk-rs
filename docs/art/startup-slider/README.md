# High-resolution options sliders

`StartupBookScrollHD.png` is an 8× adaptation of the original
`planet/Graphics.c4g/StartupBookScroll.png` 48×48 atlas. The left arrow was
generated from the original slider references with OpenAI imagegen. The right
arrow is its mirror image, and the repeating rail uses the exact cross-section
of their tails. `prepare_runtime.py` packs them into the original 3×3 facet
layout. The runtime still draws each arrow at 16×16 logical
pixels and repeats the rail over the same travel distance. It uses the
separate high-resolution full-body Wipf sprite as the thumb.

The rail matches both arrow tails pixel-for-pixel at its seams. Pressed arrows
are warm-tinted variants of the same generated arrow. The
unused atlas cells retain scaled classic artwork. Normal compatibility mode
uses this atlas on the options sliders; LegacyClonk mode keeps the original.

The original artwork is by RedWolf Design, licensed
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/).
See `planet/Graphics.c4g/COPYING` and `crates/clonk-app/assets/COPYING`.
The generated arrow source image is retained beside the preparation script so
the installed atlas is reproducible:

```sh
python3 docs/art/startup-slider/prepare_runtime.py
```

The [interactive review page](index.html) renders the old and new facets from
their actual game atlases. Drag either Wipf, click an arrow, or use the range
controls; matching sliders stay synchronized. Serve the repository root with
`python3 -m http.server 8765` and open
`http://127.0.0.1:8765/docs/art/startup-slider/` to review it.

Its [in-game comparison](in-game-comparison.png) crops both Audio sliders from
actual 3840×2160 retained GPU frames at 3× scaling. Only the slider atlas
changes between the two frames; the HD icons, tabs, Wipf, fonts, and settings
are identical. To record a new after frame, set `CLONK_HD_OPTIONS_CAPTURE` for
the `scaled_options_gpu_frame_keeps_all_nine_high_resolution_sources` test.
`build_comparison.py` combines it with a matching before frame.
