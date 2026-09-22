#!/usr/bin/env bash
set -euo pipefail

# Fit each approved preview into the occupied bounds of its 35x35 source cell.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/crates/clonk-app/assets/menu-icons"
mkdir -p "$out"

for key in menu-save menu-goals menu-rules menu-hostility menu-display \
    options-settings options-music options-fps options-audio; do
    original="$here/before/$key.png"
    preview="$here/after/$key.png"
    matte="$(mktemp "${TMPDIR:-/tmp}/clonk-menu-matte.XXXXXX.png")"
    cutout="$(mktemp "${TMPDIR:-/tmp}/clonk-menu-cutout.XXXXXX.png")"
    fitted="$(mktemp "${TMPDIR:-/tmp}/clonk-menu-fitted.XXXXXX.png")"

    if [[ "$key" == menu-hostility || "$key" == options-settings ]]; then
        read -r width height < <(magick identify -format '%w %h\n' "$preview")
        right=$((width - 1))
        bottom=$((height - 1))
        flood=( -draw 'color 0,0 floodfill' -draw "color $right,0 floodfill"
                -draw "color 0,$bottom floodfill" -draw "color $right,$bottom floodfill" )
        if [[ "$key" == options-settings ]]; then
            # The two ivory gear bores are enclosed, so the corner flood
            # cannot reach them.
            flood+=( -draw 'color 465,470 floodfill' -draw 'color 875,850 floodfill' )
        fi
        magick "$preview" -alpha on -fuzz 7% -fill none "${flood[@]}" \
            -channel A -morphology Erode Disk:1 +channel "$matte"
    else
        cp "$preview" "$matte"
    fi

    source_bounds="$(magick "$original" -alpha extract -format '%@' info:)"
    matte_bounds="$(magick "$matte" -alpha extract -format '%@' info:)"
    if [[ ! "$source_bounds" =~ ^([0-9]+)x([0-9]+)\+([0-9]+)\+([0-9]+)$ ]]; then
        printf 'invalid original bounds for %s: %s\n' "$key" "$source_bounds" >&2
        exit 1
    fi
    source_width="${BASH_REMATCH[1]}"
    source_height="${BASH_REMATCH[2]}"
    source_x="${BASH_REMATCH[3]}"
    source_y="${BASH_REMATCH[4]}"
    if [[ ! "$matte_bounds" =~ ^[0-9]+x[0-9]+\+[0-9]+\+[0-9]+$ ]]; then
        printf 'invalid preview bounds for %s: %s\n' "$key" "$matte_bounds" >&2
        exit 1
    fi
    magick "$matte" -crop "$matte_bounds" +repage "$cutout"

    target_width=$((source_width * 8))
    target_height=$((source_height * 8))
    magick "$cutout" -filter Lanczos -resize "${target_width}x${target_height}" "$fitted"
    read -r fitted_width fitted_height < <(magick identify -format '%w %h\n' "$fitted")
    x=$((source_x * 8 + (target_width - fitted_width) / 2))
    y=$((source_y * 8 + (target_height - fitted_height) / 2))
    magick -size 280x280 xc:none "$fitted" -geometry "+${x}+${y}" \
        -compose Over -composite "$out/$key.png"
    rm "$matte" "$cutout" "$fitted"
    printf '%-17s source %-14s preview %-18s runtime 280x280\n' \
        "$key" "$source_bounds" "$matte_bounds"
done

# Options.png's enabled Music, FPS and Audio cells contain the same icon plus
# a red check. Reuse the previously approved UI checkbox's isolated checkmark
# so enabled states retain the approved high-resolution base artwork.
mark="$here/support/options-checkmark.png"
mark_bounds="$(magick "$mark" -alpha extract -format '%@' info:)"
for spec in 'options-music 10 25' 'options-fps 10 25' 'options-audio 17 18'; do
    read -r key source_x source_width <<< "$spec"
    fitted="$(mktemp "${TMPDIR:-/tmp}/clonk-menu-check.XXXXXX.png")"
    magick "$mark" -crop "$mark_bounds" +repage -filter Lanczos \
        -resize "$((source_width * 8))x$((21 * 8))!" "$fitted"
    magick "$out/$key.png" "$fitted" -geometry "+$((source_x * 8))+$((14 * 8))" \
        -compose Over -composite "$out/$key-checked.png"
    rm "$fitted"
    printf '%-17s checkmark over %-17s runtime 280x280\n' "$key-checked" "$key"
done
