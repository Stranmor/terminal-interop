#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_ALACRITTY_BIN:?set TERM_INTEROP_ALACRITTY_BIN to the exact Alacritty candidate}"
e2e_backend=${TERM_INTEROP_E2E_BACKEND:-sixel}

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
preview_bin=${TERM_INTEROP_BIN:-"$project_dir/target/release/term-interop"}
runner="$script_dir/fixtures/run-preview-and-restore.sh"
zellij_layout="$script_dir/fixtures/zellij-preview.kdl"
alacritty_bin=$TERM_INTEROP_ALACRITTY_BIN
outer_columns=100
outer_rows=32

for required_command in cargo compare identify import jq magick Xvfb xdotool xdpyinfo zellij; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done
if [[ ! -x "$alacritty_bin" ]]; then
    printf 'Alacritty candidate is not executable: %s\n' "$alacritty_bin" >&2
    exit 1
fi

if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build --manifest-path "$project_dir/Cargo.toml" --release --locked -p terminal-interop-cli
fi

artifact_parent=${TERM_INTEROP_E2E_DIR:-/var/tmp}
mkdir -p "$artifact_parent"
run_dir=$(mktemp -d "$artifact_parent/terminal-interop-sixel-e2e.XXXXXX")
fixture=${TERM_INTEROP_E2E_FIXTURE:-"$run_dir/fixture.png"}

if [[ -n "${TERM_INTEROP_E2E_FIXTURE:-}" ]]; then
    if [[ ! -f "$fixture" ]]; then
        printf 'external E2E fixture is not a regular file: %s\n' "$fixture" >&2
        exit 1
    fi
else
    magick -size 1200x800 gradient:'#071426-#183f61' \
        -fill '#081221' -stroke '#38bdf8' -strokewidth 1 \
        -draw 'roundrectangle 50,50 1150,750 22,22' \
        -fill none -stroke '#1e4f70' -strokewidth 1 \
        -draw 'line 90,285 1110,285 line 90,540 1110,540 line 390,285 390,540 line 690,285 690,540 line 990,285 990,540' \
        -fill '#f8fafc' -stroke none -font DejaVu-Sans -pointsize 66 -gravity northwest \
        -annotate +90+92 'TERMINAL INTEROP' \
        -fill '#7dd3fc' -pointsize 25 -annotate +94+188 'LIVE PIXELS  /  SAME TTY  /  VERIFIED' \
        -fill '#f43f5e' -draw 'rectangle 90,330 240,500' \
        -fill '#f59e0b' -draw 'rectangle 240,330 390,500' \
        -fill '#22c55e' -draw 'rectangle 390,330 540,500' \
        -fill '#06b6d4' -draw 'rectangle 540,330 690,500' \
        -fill '#3b82f6' -draw 'rectangle 690,330 840,500' \
        -fill '#a855f7' -draw 'rectangle 840,330 990,500' \
        -fill '#e2e8f0' -stroke none -pointsize 30 -annotate +90+590 'KGP  /  SIXEL  /  SSH  /  MULTIPLEXERS' \
        -fill '#94a3b8' -pointsize 21 -annotate +92+650 '1 px geometry  |  bounded decode  |  explicit restore' \
        -fill none -stroke '#f8fafc' -strokewidth 1 \
        -draw 'rectangle 90,330 990,500 line 90,415 990,415 line 165,330 165,500 line 315,330 315,500 line 465,330 465,500 line 615,330 615,500 line 765,330 765,500 line 915,330 915,500' \
        "$fixture"
fi
fixture_geometry=$(identify -format '%w %h' "$fixture")
read -r fixture_width fixture_height <<<"$fixture_geometry"
if ((fixture_width <= 0 || fixture_height <= 0)); then
    printf 'E2E fixture has invalid geometry: %s\n' "$fixture_geometry" >&2
    exit 1
fi
fixture_sha256=$(sha256sum "$fixture" | awk '{print $1}')

display_number=
for candidate in $(seq 120 149); do
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
active_pid=
active_session=
cleanup() {
    if [[ -n "$active_pid" ]] && kill -0 "$active_pid" 2>/dev/null; then
        kill "$active_pid" 2>/dev/null || true
        wait "$active_pid" 2>/dev/null || true
    fi
    if [[ -n "$active_session" ]]; then
        zellij kill-session "$active_session" 2>/dev/null || true
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

run_case() {
    local case_name=$1
    local case_dir="$run_dir/$case_name"
    local title="terminal-interop-sixel-$case_name-$RANDOM-$$"
    local started="$case_dir/started"
    local start_fifo="$case_dir/start.fifo"
    local restored_marker="$case_dir/restored.marker"
    local live_screenshot="$case_dir/live.png"
    local restored_screenshot="$case_dir/restored.png"
    local zellij_config="$case_dir/zellij.kdl"
    local window_id=
    local live_geometry restored_geometry live_width live_height expected_width expected_height
    local crop_x crop_y cell_width cell_height
    local live_colors restored_colors live_rmse perceptual_rmse restored_rmse
    local live_metric perceptual_metric restored_metric

    mkdir -p "$case_dir"
    mkfifo "$start_fifo"
    printf 'show_startup_tips false\nshow_release_notes false\n' >"$zellij_config"

    if [[ "$case_name" == direct ]]; then
        env -u WAYLAND_DISPLAY \
            DISPLAY=":$display_number" \
            TERM_INTEROP_BIN="$preview_bin" \
            TERM_INTEROP_FIXTURE="$fixture" \
            TERM_INTEROP_RESTORED="$restored_marker" \
            TERM_INTEROP_STARTED="$started" \
            TERM_INTEROP_START_FIFO="$start_fifo" \
            TERM_INTEROP_DURATION_MS=3000 \
            TERM_INTEROP_BACKEND="$e2e_backend" \
            "$alacritty_bin" --config-file /dev/null \
                -o window.dimensions.columns="$outer_columns" \
                -o window.dimensions.lines="$outer_rows" \
                -o window.dynamic_title=false \
                -T "$title" -e "$runner" \
                >"$case_dir/alacritty.stdout" 2>"$case_dir/alacritty.stderr" &
    else
        active_session="terminal-interop-sixel-$RANDOM-$$"
        env \
            TERM_INTEROP_BIN="$preview_bin" \
            TERM_INTEROP_FIXTURE="$fixture" \
            TERM_INTEROP_RESTORED="$restored_marker" \
            TERM_INTEROP_STARTED="$started" \
            TERM_INTEROP_START_FIFO="$start_fifo" \
            TERM_INTEROP_DURATION_MS=3000 \
            TERM_INTEROP_BACKEND="$e2e_backend" \
            TERM_INTEROP_RUNNER="$runner" \
            zellij --config "$zellij_config" attach --create-background "$active_session" options \
                --default-layout "$zellij_layout" \
                --show-startup-tips false \
                --show-release-notes false
        for _ in $(seq 1 180); do
            [[ -s "$started" ]] && break
            sleep 0.05
        done
        if [[ ! -s "$started" ]]; then
            printf 'isolated Zellij preview producer did not become ready\n' >&2
            exit 1
        fi
        env -u WAYLAND_DISPLAY \
            DISPLAY=":$display_number" \
            "$alacritty_bin" --config-file /dev/null \
                -o window.dimensions.columns="$outer_columns" \
                -o window.dimensions.lines="$outer_rows" \
                -o window.dynamic_title=false \
                -T "$title" -e zellij --config "$zellij_config" attach "$active_session" \
                >"$case_dir/alacritty.stdout" 2>"$case_dir/alacritty.stderr" &
    fi
    active_pid=$!

    for _ in $(seq 1 180); do
        window_id=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
            xdotool search --name "$title" 2>/dev/null | head -1 || true)
        [[ -n "$window_id" && -s "$started" ]] && break
        sleep 0.05
    done
    if [[ -z "$window_id" || ! -s "$started" ]]; then
        printf '%s preview consumer did not become ready\n' "$case_name" >&2
        exit 1
    fi
    if [[ "$case_name" == zellij ]]; then
        for _ in $(seq 1 180); do
            if zellij --config "$zellij_config" --session "$active_session" \
                action list-clients >/dev/null 2>&1; then
                break
            fi
            sleep 0.05
        done
        # The client acknowledgement and the final PTY resize are separate events.
        sleep 0.5
    fi

    printf 'render\n' >"$start_fifo"
    sleep 1
    env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
        import -window "$window_id" "$live_screenshot"

    for _ in $(seq 1 140); do
        [[ -s "$restored_marker" ]] && break
        sleep 0.05
    done
    if [[ ! -s "$restored_marker" ]]; then
        printf '%s preview did not restore its caller before the deadline\n' "$case_name" >&2
        exit 1
    fi
    sleep 0.2
    env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
        import -window "$window_id" "$restored_screenshot"
    wait "$active_pid"
    active_pid=
    active_session=

    live_geometry=$(identify -format '%wx%h' "$live_screenshot")
    restored_geometry=$(identify -format '%wx%h' "$restored_screenshot")
    live_width=${live_geometry%x*}
    live_height=${live_geometry#*x}
    expected_width=$live_width
    crop_x=0
    crop_y=0
    if [[ "$case_name" == zellij ]]; then
        if ((live_width % outer_columns != 0 || live_height % outer_rows != 0)); then
            printf '%s framebuffer is not an integral terminal cell grid: %s\n' \
                "$case_name" "$live_geometry" >&2
            exit 1
        fi
        cell_width=$((live_width / outer_columns))
        cell_height=$((live_height / outer_rows))
        expected_width=$((live_width - (2 * cell_width)))
        crop_x=$cell_width
        crop_y=$cell_height
    fi
    expected_height=$((expected_width * fixture_height / fixture_width))
    if ((crop_x + expected_width > live_width || crop_y + expected_height > live_height)); then
        printf '%s framebuffer is too small for the expected raster region\n' "$case_name" >&2
        exit 1
    fi

    magick "$live_screenshot" \
        -crop "${expected_width}x${expected_height}+${crop_x}+${crop_y}" +repage \
        "$case_dir/live-image.png"
    magick "$restored_screenshot" \
        -crop "${expected_width}x${expected_height}+${crop_x}+${crop_y}" +repage \
        "$case_dir/restored-image.png"
    magick "$fixture" -resize "${expected_width}x${expected_height}!" "$case_dir/expected.png"
    live_metric=$(compare -metric RMSE \
        "$case_dir/expected.png" "$case_dir/live-image.png" null: 2>&1 || true)
    magick "$case_dir/expected.png" -blur '0x1.5' "$case_dir/expected-perceptual.png"
    magick "$case_dir/live-image.png" -blur '0x1.5' "$case_dir/live-perceptual.png"
    perceptual_metric=$(compare -metric RMSE \
        "$case_dir/expected-perceptual.png" "$case_dir/live-perceptual.png" null: 2>&1 || true)
    restored_metric=$(compare -metric RMSE \
        "$case_dir/expected.png" "$case_dir/restored-image.png" null: 2>&1 || true)
    live_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$live_metric")
    perceptual_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$perceptual_metric")
    restored_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$restored_metric")
    live_colors=$(identify -format '%k' "$live_screenshot")
    restored_colors=$(identify -format '%k' "$restored_screenshot")

    if ((live_colors < 500)); then
        printf '%s live Sixel preview retained only %s colors\n' "$case_name" "$live_colors" >&2
        exit 1
    fi
    # The Zellij path can shift a cell-bounded raster by one or two physical pixels while retaining
    # the complete image. Keep a strict raw bound for palette/geometry loss and use the independent
    # blurred metric below to reject perceptual degradation.
    if [[ -z "$live_rmse" ]] || ! awk -v value="$live_rmse" 'BEGIN { exit !(value < 0.18) }'; then
        printf '%s framebuffer palette or geometry diverges from the fixture: RMSE=%s\n' \
            "$case_name" "${live_rmse:-unknown}" >&2
        exit 1
    fi
    if [[ -z "$perceptual_rmse" ]] \
        || ! awk -v value="$perceptual_rmse" 'BEGIN { exit !(value < 0.12) }'; then
        printf '%s framebuffer loses perceptual image structure: RMSE=%s\n' \
            "$case_name" "${perceptual_rmse:-unknown}" >&2
        exit 1
    fi
    if [[ -z "$restored_rmse" ]] || ! awk -v value="$restored_rmse" 'BEGIN { exit !(value > 0.20) }'; then
        printf '%s restore still resembles the raster fixture: RMSE=%s\n' \
            "$case_name" "${restored_rmse:-unknown}" >&2
        exit 1
    fi
    if [[ -s "$case_dir/alacritty.stderr" ]]; then
        printf '%s Alacritty emitted errors:\n' "$case_name" >&2
        sed -n '1,160p' "$case_dir/alacritty.stderr" >&2
        exit 1
    fi

    jq -n \
        --arg case "$case_name" \
        --arg live_screenshot "$live_screenshot" \
        --arg restored_screenshot "$restored_screenshot" \
        --arg live_geometry "$live_geometry" \
        --arg restored_geometry "$restored_geometry" \
        --argjson crop_x "$crop_x" \
        --argjson crop_y "$crop_y" \
        --argjson image_width "$expected_width" \
        --argjson image_height "$expected_height" \
        --argjson live_colors "$live_colors" \
        --argjson restored_colors "$restored_colors" \
        --argjson live_rmse "$live_rmse" \
        --argjson perceptual_rmse "$perceptual_rmse" \
        --argjson restored_rmse "$restored_rmse" \
        '{
            case: $case,
            live: {
                screenshot: $live_screenshot,
                geometry: $live_geometry,
                image_region: {
                    x: $crop_x,
                    y: $crop_y,
                    width: $image_width,
                    height: $image_height
                },
                unique_colors: $live_colors,
                normalized_rmse_to_fixture: $live_rmse,
                perceptual_rmse_to_fixture: $perceptual_rmse
            },
            restored: {
                screenshot: $restored_screenshot,
                geometry: $restored_geometry,
                unique_colors: $restored_colors,
                normalized_rmse_to_fixture: $restored_rmse
            }
        }' >"$case_dir/summary.json"
}

run_case direct
run_case zellij

jq -s \
    --arg schema 'urn:terminal-interop:sixel-preview-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg alacritty_version "$($alacritty_bin --version)" \
    --arg zellij_version "$(zellij --version)" \
    --arg fixture "$fixture" \
    --arg fixture_sha256 "$fixture_sha256" \
    --argjson fixture_width "$fixture_width" \
    --argjson fixture_height "$fixture_height" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        implementations: {
            outer_terminal: $alacritty_version,
            multiplexer: $zellij_version
        },
        fixture: {
            path: $fixture,
            sha256: $fixture_sha256,
            width: $fixture_width,
            height: $fixture_height
        },
        renderer: "sixel",
        cases: .
    }' "$run_dir/direct/summary.json" "$run_dir/zellij/summary.json" \
    | tee "$run_dir/summary.json"

printf 'SIXEL_E2E_ARTIFACTS=%s\n' "$run_dir"
