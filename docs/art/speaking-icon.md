# Speaking icon

`crates/clonk-app/assets/Speaking.png` is a 320×320 RGBA super-resolution
adaptation of the original 40×40 sound icon in `GUIIcons.png`: phase 23,
rectangle `(200, 120, 40, 40)`. It retains the silver speaker and two gold
sound-wave arcs, with a refined rear basket and magnet connection for more
realistic construction. The talking overlay still occupies 20×20 logical pixels.

Source artwork: RedWolf Design and Jonathan Veit (AniProGuy).
License: [CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/),
as recorded in `crates/clonk-app/assets/COPYING`.

The app embeds and caches this port-owned voice-chat asset. The legacy
`planet` resource tree remains identical to the presentation oracle's pinned
inputs. The renderer can still use the atlas sound icon as a fallback when
no standalone sprite is supplied.

Created using the built-in OpenAI imagegen tool on 2026-09-21. The reference
inputs were the original cell and a 640×640 enlargement of that cell.
After the initial enlargement, a second edit refined the speaker's rear
assembly and material rendering. Both stages used imagegen to obtain transparent
alpha. The selected final output was reduced to 320×320 with macOS `sips`.
The legacy atlas is unchanged.

Initial super-resolution prompt:

> Faithfully super-resolve this exact game icon. Image 1 is an enlarged view of the original 40x40 sprite; Image 2 is that original at native resolution. Return this same complete icon at high resolution with smooth edges and genuine transparent alpha. Preserve precisely the original layout, proportions, perspective, framing, silhouette, colors, shadows and highlights. It is a silver/gray speaker occupying the left and center, with TWO GOLD SOUND-WAVE ARCS on its lower right; both gold arcs are essential parts of the icon and must remain in exactly their original positions, sizes and curves. Preserve the rear metal casing at left, central domed cone and surrounding rings exactly where they are. Only reconstruct smooth detail implied by the pixels. Do not add mounting holes, tabs, a foot or any new features. Do not redesign the speaker. Do not remove, shift or redesign the gold arcs. Keep original margins and crop. Make the background genuinely transparent, with no checkerboard, backdrop or shadow outside the original silhouette. This is a resolution restoration of an existing sprite, not an invitation to make a new icon.

Initial transparency correction prompt:

> Make a transparent-background PNG cutout of this exact speaker-and-two-gold-arcs icon. Remove the black background completely. Outside the icon must be transparent alpha, not black or a checkerboard. Preserve the existing icon design, composition and color.

Geometry and materials refinement prompt:

> Use case: precise-object-edit
> Asset type: realistic transparent game UI sprite.
> Edit the supplied speaker-and-two-gold-sound-wave-arcs icon. The user likes its established design and composition, but the rear housing does not connect plausibly to the front. Correct that construction and make the speaker look like a physically real manufactured loudspeaker.
>
> Primary correction: reconstruct the visible rear assembly at upper left as a coherent loudspeaker basket and magnet. A rigid, tapering metal basket must connect the back of the front mounting rim to a smaller rear magnet assembly; show a few sturdy support ribs and modest ventilation openings where the existing rear silhouette permits. Align the magnet, basket, cone, and domed dust cap along one consistent three-dimensional axis perpendicular to the front rim. All elliptical cross-sections must agree in perspective. Give the junction real thickness, attachment, occlusion and contact shadows, so it does not look like a second cylinder pasted beside a flat front disc. Keep the rear assembly within roughly the current upper-left footprint; no exploded parts or cutaway.
>
> Realism: physically credible brushed metal, a subtly textured silver cone, domed dust cap, dark rubber roll surround, and realistic reflections under the current soft upper-left light. Preserve the silver, charcoal and restrained warm gold palette, including the existing gold detail around the dust cap. Keep small-scale readability; avoid excessive tiny detail.
>
> Invariants: keep the same front-facing three-quarter angle, overall icon framing, front rim shape and location, cone center and proportions, and the exact two gold sound-wave arcs at lower right with their existing placement, shape and color. No new sound waves, text, branding, badge, stand, cables, screws on the front face, or surroundings. The sound waves remain symbolic icon elements; the speaker itself should look physically real.
>
> Output: one isolated square PNG icon with genuine transparent alpha outside the speaker and gold arcs. Preserve transparency, with clean antialiased edges and no colored fringe. No black backdrop, no checkerboard baked into pixels, no ground shadow.

Final transparency correction prompt:

> Create a transparent PNG cutout of this exact realistic loudspeaker and its two gold sound-wave arcs. Remove the gray-and-white checkerboard completely; it is unwanted background, not part of the artwork. Use a genuine RGBA alpha channel with zero opacity outside the icon. Keep the new vented metal basket, magnet, front cone, perspective, gold arcs and their positions unchanged. Clean antialiased edges, no backdrop, no checkerboard pixels, no colored fringe.
