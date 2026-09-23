# Navigation-arrow review, batch 6

The sources are `planet/Graphics.c4g/Arrow.png` (four 64×64 red arrow phases) and `planet/Graphics.c4g/GUIBigArrows.png` (four 19×40 wooden arrow phases: left, right, left pressed, right pressed). RedWolf Design authored the original content under [CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/), as documented in `planet/Graphics.c4g/COPYING`.

The eight proposals are adaptations made with OpenAI's built-in image generation model. Each original cell controlled its corresponding output; the first generated cell from each sheet also provided a style reference for the remaining phases. `prepare_images.py` removes baked checkerboards where present, reconstructs transparent antialiased edges, and fits each cutout to its source cell. The red arrows also retain the source drop shadow. The resulting previews are 8× the source dimensions. `game-size/` shows each result downsampled to the original cell size, while `support/` retains the raw generated outputs.

The color and shading pass regenerated the normal right wooden arrow using the normal left arrow as a material and lighting reference. Its old raw generation is retained as `support/wood-arrow-1-generated-v1.png`. The red arrows were reviewed together at full and game size; their crimson hue, restrained bevels, and shadow treatment remain consistent, so their art was kept.

All eight icons were approved after the color and shading pass. They ship as embedded app assets that replace the corresponding cells at render time; the original sheets and their geometry stay intact. The pull request description shows the before/after comparison.
