# Complete checkbox icon revision

The approved 256×256 checkbox states adapt RedWolf Design's 32×32
`planet/Graphics.c4g/GUICheckbox.png` checked and unchecked cells. They retain
the flat square box, lower-right shadow, broad red V, blunt left nose, and
boxy upper-right arm. The source artwork and adaptations use CC BY-NC 4.0;
see `crates/clonk-app/assets/COPYING`.

The complete checked cell, an enlarged crop of its left arm, and the previously
approved flat unchecked box were supplied to built-in OpenAI imagegen. The
selected prompt was:

> Use case: precise-object-edit. Asset type: complete checked checkbox game
> icon, 256×256. Image 1 is the ORIGINAL COMPLETE 32×32 checked icon and is
> the authoritative EDIT TARGET for the ENTIRE composition and checkmark
> silhouette. Image 2 is an enlarged close-up from that same original; it is
> a DETAIL REFERENCE only for the red checkmark's left end. Image 3 is the
> approved high-resolution flat gray unchecked checkbox; use it as the box's
> appearance and exact shape. Produce a faithful smooth super-resolution of
> the COMPLETE checked icon in Image 1, with the full gray box and entire red
> checkmark visible on a square transparent canvas. Preserve Image 1's
> overall position, scale, and broad V-shaped red mark. The left end must
> follow Image 2: short blunt vertical face, angular rising top shoulder,
> thick red body, dark diagonal underside. It must be integral to the
> checkmark, not a separate narrow horizontal tab and not a pointed
> triangular nose. The right rising arm ends with the original's squared,
> nearly vertical outer edge. Keep the gray box flat, square, and understated
> as in Image 3, with a restrained lower-right shadow. Smooth red painted
> shading and clean antialiased edges; keep the original's shapes and
> proportions. Genuine transparent alpha outside the complete icon. No
> enlarged pixel blocks, checkerboard, text, extra objects, isolated crop,
> bevelled metal box, or different checkmark design.

The selected raw result is [`full-icon-imagegen.png`](full-icon-imagegen.png).
It contains a baked checkerboard and a changed box finish. The approved red
mark in `approved/checkmark.png` was extracted from that result, scaled to the
classic red bounds, and fitted at its blunt left end to the original rows.
It was composited over the approved smooth flat box in
`approved/checkbox-off.png`. `approved/checkbox-on.png` is the complete icon
the user reviewed; `approved/checkbox-disabled.png` uses Rec. 709 luma on
the same mark. [`prepare_runtime.py`](prepare_runtime.py) verifies those exact
relationships and copies the approved sprites into the application.
