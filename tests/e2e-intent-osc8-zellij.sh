#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_ALACRITTY_BIN:?set TERM_INTEROP_ALACRITTY_BIN to the exact Alacritty candidate}"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(cd -- "$script_dir/.." && pwd)
runner="$script_dir/fixtures/emit-osc8-intent-link.sh"
layout="$script_dir/fixtures/zellij-osc8-intent.kdl"
zellij_config="$script_dir/fixtures/zellij-osc8-intent-config.kdl"
alacritty_bin=$TERM_INTEROP_ALACRITTY_BIN

for required_command in cargo jq Xvfb xdotool xdpyinfo zellij; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

if [[ ! -x "$alacritty_bin" ]]; then
    printf 'Alacritty candidate is not executable: %s\n' "$alacritty_bin" >&2
    exit 1
fi

cargo build --release --locked --manifest-path "$repo_dir/Cargo.toml" \
    -p terminal-interop-cli >/dev/null
term_interop=${TERM_INTEROP_BIN:-$repo_dir/target/release/term-interop}

run_dir=$(mktemp -d /var/tmp/terminal-interop-osc8-zellij.XXXXXX)
fixture=${TERM_INTEROP_TEST_FIXTURE:-$run_dir/test-photo.png}
if [[ -z "${TERM_INTEROP_TEST_FIXTURE:-}" ]]; then
    printf 'terminal-interop OSC 8 callback fixture\n' >"$fixture"
elif [[ ! -f "$fixture" ]]; then
    printf 'test fixture is absent: %s\n' "$fixture" >&2
    exit 1
fi

listener_output="$run_dir/listener.jsonl"
listener_error="$run_dir/listener.stderr"
alacritty_output="$run_dir/alacritty.stdout"
alacritty_error="$run_dir/alacritty.stderr"
screen_dump="$run_dir/screen.txt"
xvfb_log="$run_dir/xvfb.log"
session="terminal-interop-osc8-$RANDOM-$$"
title="terminal-interop-osc8-$RANDOM-$$"

endpoint=$($term_interop intent endpoint)
uri=$($term_interop intent uri "$endpoint" "$fixture")

$term_interop intent listen --once "$endpoint" >"$listener_output" 2>"$listener_error" &
listener_pid=$!

display_number=
for candidate in $(seq 220 249); do
    if [[ ! -e "/tmp/.X11-unix/X$candidate" ]]; then
        display_number=$candidate
        break
    fi
done
if [[ -z "$display_number" ]]; then
    printf 'cannot allocate an isolated X11 display\n' >&2
    exit 1
fi
display=":$display_number"

Xvfb "$display" -screen 0 1200x800x24 -nolisten tcp >"$xvfb_log" 2>&1 &
xvfb_pid=$!
alacritty_pid=

cleanup() {
    if [[ -n "$alacritty_pid" ]] && kill -0 "$alacritty_pid" 2>/dev/null; then
        kill "$alacritty_pid" 2>/dev/null || true
        wait "$alacritty_pid" 2>/dev/null || true
    fi
    zellij --config "$zellij_config" delete-session "$session" --force >/dev/null 2>&1 || true
    if kill -0 "$listener_pid" 2>/dev/null; then
        kill "$listener_pid" 2>/dev/null || true
        wait "$listener_pid" 2>/dev/null || true
    fi
    kill "$xvfb_pid" 2>/dev/null || true
    wait "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 100); do
    if env -u WAYLAND_DISPLAY DISPLAY="$display" xdpyinfo >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done

for _ in $(seq 1 100); do
    if [[ -s "$listener_output" ]]; then
        break
    fi
    sleep 0.05
done
if ! jq -e --arg endpoint "$endpoint" \
    'select(.schema == "urn:terminal-interop:intent-ready:v1" and .endpoint == $endpoint)' \
    <"$listener_output" >/dev/null; then
    printf 'private listener did not emit its bound ready receipt\n' >&2
    exit 1
fi

env \
    TERM_INTEROP_TEST_URI="$uri" \
    TERM_INTEROP_TEST_RUNNER="$runner" \
    zellij --config "$zellij_config" attach --create-background "$session" options \
        --default-layout "$layout" \
        --show-startup-tips false \
        --show-release-notes false \
        --pane-frames false \
        --session-serialization false

env -u WAYLAND_DISPLAY \
    DISPLAY="$display" \
    "$alacritty_bin" --config-file /dev/null \
        -o window.dimensions.columns=60 \
        -o window.dimensions.lines=24 \
        -o window.dynamic_title=false \
        -o 'window.decorations="None"' \
        -o window.padding.x=0 \
        -o window.padding.y=0 \
        -T "$title" \
        -e zellij --config "$zellij_config" attach "$session" \
        >"$alacritty_output" 2>"$alacritty_error" &
alacritty_pid=$!

window_id=
for _ in $(seq 1 200); do
    window_id=$(env -u WAYLAND_DISPLAY DISPLAY="$display" \
        xdotool search --name "$title" 2>/dev/null | head -1 || true)
    zellij --config "$zellij_config" --session "$session" action dump-screen \
        >"$screen_dump" 2>/dev/null || true
    if [[ -n "$window_id" ]] && grep -q 'OSC8_INTENT_LINK_READY' "$screen_dump"; then
        break
    fi
    sleep 0.05
done
if [[ -z "$window_id" ]] || ! grep -q 'OSC8_INTENT_LINK_READY' "$screen_dump"; then
    printf 'isolated OSC 8 link consumer did not become ready\n' >&2
    exit 1
fi

link_line=$(grep -n 'test-photo\.png' "$screen_dump" | head -1 | cut -d: -f1)
link_text=$(sed -n "${link_line}p" "$screen_dump")
link_column=$(awk '{ print index($0, "test-photo.png") }' <<<"$link_text")
if ((link_line < 1 || link_column < 1)); then
    printf 'could not map the OSC 8 link to terminal cells\n' >&2
    exit 1
fi

geometry=$(env -u WAYLAND_DISPLAY DISPLAY="$display" \
    xdotool getwindowgeometry --shell "$window_id")
width=$(awk -F= '$1 == "WIDTH" {print $2}' <<<"$geometry")
height=$(awk -F= '$1 == "HEIGHT" {print $2}' <<<"$geometry")

panes=$(zellij --config "$zellij_config" --session "$session" action \
    list-panes --json --geometry --state)
terminal_panes=$(jq '[.[] | select(.is_plugin == false and .exited == false)] | length' \
    <<<"$panes")
if ((terminal_panes != 1)); then
    printf 'expected exactly one live terminal pane, found %s\n' "$terminal_panes" >&2
    exit 1
fi

pane_content_x=$(jq -r \
    '[.[] | select(.is_plugin == false and .exited == false)][0].pane_content_x' \
    <<<"$panes")
pane_content_y=$(jq -r \
    '[.[] | select(.is_plugin == false and .exited == false)][0].pane_content_y' \
    <<<"$panes")
pane_content_columns=$(jq -r \
    '[.[] | select(.is_plugin == false and .exited == false)][0].pane_content_columns' \
    <<<"$panes")
pane_content_rows=$(jq -r \
    '[.[] | select(.is_plugin == false and .exited == false)][0].pane_content_rows' \
    <<<"$panes")

if ((link_column > pane_content_columns || link_line > pane_content_rows)); then
    printf 'OSC 8 link cell lies outside the reported Zellij pane content geometry\n' >&2
    exit 1
fi

# dump-screen coordinates are relative to pane content, while xdotool clicks are
# relative to the full terminal grid. Translate through Zellij's exact content
# origin so tab bars and pane frames cannot shift the physical click target.
click_x=$(((2 * (pane_content_x + link_column - 1) + 1) * width / (2 * 60)))
click_y=$(((2 * (pane_content_y + link_line - 1) + 1) * height / (2 * 24)))

env -u WAYLAND_DISPLAY DISPLAY="$display" xdotool windowfocus "$window_id"
env -u WAYLAND_DISPLAY DISPLAY="$display" xdotool keydown Shift_L
env -u WAYLAND_DISPLAY DISPLAY="$display" \
    xdotool mousemove --window "$window_id" "$click_x" "$click_y"
sleep 0.2
env -u WAYLAND_DISPLAY DISPLAY="$display" xdotool click 1 keyup Shift_L

for _ in $(seq 1 120); do
    if [[ $(wc -l <"$listener_output") -ge 2 ]]; then
        break
    fi
    sleep 0.05
done

intent=$(sed -n '2p' "$listener_output")
if ! jq -e --arg endpoint "$endpoint" \
    'select(.schema == "urn:terminal-interop:open-intent:v1"
        and .endpoint == $endpoint
        and .action == "open_artifact"
        and .target.encoding == "unix_bytes_base64url_v1")' \
    <<<"$intent" >/dev/null; then
    printf 'physical Shift+click did not reach the bound listener\n' >&2
    if [[ -s "$listener_error" ]]; then
        sed -n '1,120p' "$listener_error" >&2
    fi
    exit 1
fi

if [[ -s "$alacritty_error" ]]; then
    printf 'Alacritty emitted errors during OSC 8 click E2E\n' >&2
    sed -n '1,120p' "$alacritty_error" >&2
    exit 1
fi

jq -n \
    --arg schema 'urn:terminal-interop:osc8-zellij-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg terminal "$($alacritty_bin --version)" \
    --arg multiplexer "$(zellij --version)" \
    --arg endpoint "$endpoint" \
    --arg path "$fixture" \
    --arg cell "$link_column,$link_line" \
    --arg pixel "$click_x,$click_y" \
    --arg pane_origin "$pane_content_x,$pane_content_y" \
    --arg pane_size "$pane_content_columns,$pane_content_rows" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        consumer: { terminal: $terminal, multiplexer: $multiplexer },
        click: {
            gesture: "Shift+Button1",
            pane_content_origin: $pane_origin,
            pane_content_size: $pane_size,
            cell: $cell,
            pixel: $pixel
        },
        callback: { endpoint: $endpoint, path: $path, receipt: true }
    }' | tee "$run_dir/summary.json"

printf 'OSC8_ZELLIJ_E2E_ARTIFACTS=%s\n' "$run_dir"
