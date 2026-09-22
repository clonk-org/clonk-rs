#!/usr/bin/env bash
set -euo pipefail

# Prepare the nine approved previews without redrawing or distorting the art.
# Keep the original PNG canvas at 8x size so its HUD layout stays unchanged.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/crates/clonk-app/assets/hud-icons"
mkdir -p "$out"

for key in build captain construction energy exit magic player score wealth; do
    original="$here/before/$key.png"
    preview="$here/after/$key.png"
    read -r width height < <(magick identify -format '%w %h\n' "$original")
    read -r preview_width preview_height < <(magick identify -format '%w %h\n' "$preview")
    right=$((preview_width - 1))
    bottom=$((preview_height - 1))

    # The generated background is connected to the four corners. Flood only
    # that matte, retaining enclosed light surfaces such as the mallet head.
    matte="$(mktemp "${TMPDIR:-/tmp}/clonk-hud-matte.XXXXXX.png")"
    cutout="$(mktemp "${TMPDIR:-/tmp}/clonk-hud-cutout.XXXXXX.png")"
    fitted="$(mktemp "${TMPDIR:-/tmp}/clonk-hud-fitted.XXXXXX.png")"
    magick "$preview" -alpha on -fuzz 7% -fill none \
        -draw 'color 0,0 floodfill' -draw "color $right,0 floodfill" \
        -draw "color 0,$bottom floodfill" \
        -draw "color $right,$bottom floodfill" "$matte"

    source_bounds="$(magick "$original" -alpha extract -format '%@' info:)"
    matte_bounds="$(magick "$matte" -alpha extract -format '%@' info:)"
    if [[ ! "$source_bounds" =~ ^([0-9]+)x([0-9]+)\+([0-9]+)\+([0-9]+)$ ]]; then
        printf 'invalid original bounds: %s\n' "$source_bounds" >&2
        exit 1
    fi
    source_width="${BASH_REMATCH[1]}"
    source_height="${BASH_REMATCH[2]}"
    source_x="${BASH_REMATCH[3]}"
    source_y="${BASH_REMATCH[4]}"
    if [[ ! "$matte_bounds" =~ ^([0-9]+)x([0-9]+)\+([0-9]+)\+([0-9]+)$ ]]; then
        printf 'invalid preview bounds: %s\n' "$matte_bounds" >&2
        exit 1
    fi

    target_width=$((source_width * 8))
    target_height=$((source_height * 8))
    canvas_width=$((width * 8))
    canvas_height=$((height * 8))
    magick "$matte" -crop "$matte_bounds" +repage \
        -channel A -morphology Erode Disk:1 +channel \
        -background none -alpha background "$cutout"
    magick "$cutout" -filter Lanczos \
        -resize "${target_width}x${target_height}" "$fitted"
    read -r fitted_width fitted_height < <(magick identify -format '%w %h\n' "$fitted")
    x=$((source_x * 8 + (target_width - fitted_width) / 2))
    y=$((source_y * 8 + (target_height - fitted_height) / 2))
    magick -size "${canvas_width}x${canvas_height}" xc:none \
        "$fitted" -geometry "+${x}+${y}" -compose Over -composite \
        "$out/$key.png"
    rm "$matte" "$cutout" "$fitted"
    printf '%-13s %4sx%-4s  source %-15s preview %-18s\n' \
        "$key" "$canvas_width" "$canvas_height" "$source_bounds" "$matte_bounds"
done
