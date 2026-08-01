#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
preview_bin=${TERM_INTEROP_BIN:-"$project_dir/target/release/term-interop"}
runner="$script_dir/fixtures/run-preview-and-restore.sh"

for required_command in cargo compare identify import jq kitty magick Xvfb xdotool xdpyinfo; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build --manifest-path "$project_dir/Cargo.toml" --release --locked -p terminal-interop-cli
fi

artifact_parent=${TERM_INTEROP_E2E_DIR:-/var/tmp}
mkdir -p "$artifact_parent"
run_dir=$(mktemp -d "$artifact_parent/terminal-interop-preview-e2e.XXXXXX")
fixture="$run_dir/fixture.png"
live_screenshot="$run_dir/live.png"
restored_screenshot="$run_dir/restored.png"
restored_marker="$run_dir/restored.marker"

magick -size 960x640 gradient:'#10162c-#f0b35a' \
    -stroke '#55d6ff' -strokewidth 2 -fill none \
    -draw 'rectangle 40,40 920,600 line 40,320 920,320 line 480,40 480,600' \
    -fill '#f8fafc' -stroke none -pointsize 42 -gravity center \
    -annotate +0+0 'TERMINAL INTEROP 1px DETAIL' \
    "$fixture"

display_number=
for candidate in $(seq 90 119); do
    if [[ ! -e "/tmp/.X11-unix/X$candidate" ]]; then
        display_number=$candidate
        break
    fi
done
if [[ -z "$display_number" ]]; then
    printf 'cannot allocate an isolated X11 display\n' >&2
    exit 1
fi

Xvfb ":$display_number" -screen 0 1400x900x24 -nolisten tcp >"$run_dir/xvfb.log" 2>&1 &
xvfb_pid=$!
kitty_pid=
cleanup() {
    if [[ -n "$kitty_pid" ]] && kill -0 "$kitty_pid" 2>/dev/null; then
        kill "$kitty_pid" 2>/dev/null || true
        wait "$kitty_pid" 2>/dev/null || true
    fi
    kill "$xvfb_pid" 2>/dev/null || true
    wait "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 100); do
    if env -u WAYLAND_DISPLAY DISPLAY=":$display_number" xdpyinfo >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done

unique_title="terminal-interop-preview-$RANDOM-$$"
env -u WAYLAND_DISPLAY \
    DISPLAY=":$display_number" \
    TERM_INTEROP_BIN="$preview_bin" \
    TERM_INTEROP_FIXTURE="$fixture" \
    TERM_INTEROP_RESTORED="$restored_marker" \
    TERM_INTEROP_DURATION_MS=3000 \
    TERM_INTEROP_BACKEND=kgp \
    kitty --config NONE --title "$unique_title" \
        --override initial_window_width=100c \
        --override initial_window_height=32c \
        --override remember_window_size=no \
        -- "$runner" \
        >"$run_dir/preview.stdout" 2>"$run_dir/preview.stderr" &
kitty_pid=$!

window_id=
for _ in $(seq 1 160); do
    window_id=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
        xdotool search --name "$unique_title" 2>/dev/null | head -1 || true)
    [[ -n "$window_id" ]] && break
    sleep 0.05
done
if [[ -z "$window_id" ]]; then
    printf 'preview window was not observed\n' >&2
    exit 1
fi

sleep 1
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    import -window "$window_id" "$live_screenshot"

for _ in $(seq 1 120); do
    [[ -s "$restored_marker" ]] && break
    sleep 0.05
done
if [[ ! -s "$restored_marker" ]]; then
    printf 'preview did not restore its caller before the deadline\n' >&2
    exit 1
fi

sleep 0.2
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    import -window "$window_id" "$restored_screenshot"
wait "$kitty_pid"
kitty_pid=

live_colors=$(identify -format '%k' "$live_screenshot")
restored_colors=$(identify -format '%k' "$restored_screenshot")
live_geometry=$(identify -format '%wx%h' "$live_screenshot")
restored_geometry=$(identify -format '%wx%h' "$restored_screenshot")
live_width=${live_geometry%x*}
expected_height=$((live_width * 640 / 960))
magick "$live_screenshot" -crop "${live_width}x${expected_height}+0+0" +repage \
    "$run_dir/live-image.png"
magick "$fixture" -resize "${live_width}x${expected_height}!" "$run_dir/expected.png"
rmse_output=$(compare -metric RMSE \
    "$run_dir/expected.png" "$run_dir/live-image.png" null: 2>&1 || true)
normalized_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$rmse_output")

if ((live_colors < 1000)); then
    printf 'live preview retained only %s colors\n' "$live_colors" >&2
    exit 1
fi
if ((restored_colors >= live_colors)); then
    printf 'alternate-screen restore did not reduce framebuffer complexity\n' >&2
    exit 1
fi
if [[ -z "$normalized_rmse" ]] || ! awk -v value="$normalized_rmse" 'BEGIN { exit !(value < 0.05) }'; then
    printf 'live framebuffer does not match the raster fixture closely enough: RMSE=%s\n' \
        "${normalized_rmse:-unknown}" >&2
    exit 1
fi
if [[ -s "$run_dir/preview.stderr" ]]; then
    printf 'preview emitted errors:\n' >&2
    sed -n '1,120p' "$run_dir/preview.stderr" >&2
    exit 1
fi

jq -n \
    --arg schema 'urn:terminal-interop:preview-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg kitty_version "$(kitty --version)" \
    --arg live_screenshot "$live_screenshot" \
    --arg restored_screenshot "$restored_screenshot" \
    --arg live_geometry "$live_geometry" \
    --arg restored_geometry "$restored_geometry" \
    --argjson normalized_rmse "$normalized_rmse" \
    --argjson live_colors "$live_colors" \
    --argjson restored_colors "$restored_colors" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        implementation: $kitty_version,
        renderer: "kgp",
        live: {
            screenshot: $live_screenshot,
            geometry: $live_geometry,
            unique_colors: $live_colors,
            normalized_rmse_to_fixture: $normalized_rmse
        },
        restored: {
            screenshot: $restored_screenshot,
            geometry: $restored_geometry,
            unique_colors: $restored_colors
        }
    }' | tee "$run_dir/summary.json"

printf 'PREVIEW_E2E_ARTIFACTS=%s\n' "$run_dir"
