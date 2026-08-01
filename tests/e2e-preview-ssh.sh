#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_SSH_TARGET:?set TERM_INTEROP_SSH_TARGET to a BatchMode-capable SSH target}"
: "${TERM_INTEROP_ALACRITTY_BIN:?set TERM_INTEROP_ALACRITTY_BIN to the exact local Sixel terminal}"

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
runner="$script_dir/fixtures/run-preview-and-restore.sh"
ssh_target=$TERM_INTEROP_SSH_TARGET
alacritty_bin=$TERM_INTEROP_ALACRITTY_BIN
ssh_options=(-o BatchMode=yes -o ConnectTimeout=10)

for required_command in compare identify import jq kitty magick scp ssh Xvfb xdotool xdpyinfo; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done
if [[ ! -x "$alacritty_bin" ]]; then
    printf 'Sixel terminal is not executable: %s\n' "$alacritty_bin" >&2
    exit 1
fi

remote_bin=${TERM_INTEROP_SSH_BIN:-}
if [[ -z "$remote_bin" ]]; then
    remote_bin=$(ssh "${ssh_options[@]}" "$ssh_target" 'command -v term-interop')
fi
remote_bin=${remote_bin//$'\r'/}
if [[ ! "$remote_bin" =~ ^/[A-Za-z0-9._/+:-]+$ ]]; then
    printf 'remote term-interop path must be an absolute shell-safe path: %s\n' "$remote_bin" >&2
    exit 1
fi
if ! ssh "${ssh_options[@]}" "$ssh_target" test -x "$remote_bin"; then
    printf 'remote term-interop is not executable: %s\n' "$remote_bin" >&2
    exit 1
fi

artifact_parent=${TERM_INTEROP_E2E_DIR:-/var/tmp}
mkdir -p "$artifact_parent"
run_dir=$(mktemp -d "$artifact_parent/terminal-interop-ssh-e2e.XXXXXX")
image_fixture="$run_dir/fixture.png"
text_fixture="$run_dir/fixture.txt"

magick -size 960x640 gradient:'#10162c-#f0b35a' \
    -stroke '#55d6ff' -strokewidth 2 -fill none \
    -draw 'rectangle 40,40 920,600 line 40,320 920,320 line 480,40 480,600' \
    -fill '#f8fafc' -stroke none -pointsize 42 -gravity center \
    -annotate +0+0 'SSH SIXEL 1px DETAIL' \
    "$image_fixture"
printf 'SSH TEXT PREVIEW SAFE\nescape follows: \033[31mNOT_RED\nwide text: Привет 世界\n' \
    >"$text_fixture"

remote_dir=$(ssh "${ssh_options[@]}" "$ssh_target" \
    'umask 077; mktemp -d "${TMPDIR:-/tmp}/terminal-interop-ssh-e2e.XXXXXX"')
remote_dir=${remote_dir//$'\r'/}
if [[ ! "$remote_dir" =~ ^/[A-Za-z0-9._/+:-]*/terminal-interop-ssh-e2e\.[A-Za-z0-9]+$ ]]; then
    printf 'remote temporary directory failed the safety check: %s\n' "$remote_dir" >&2
    exit 1
fi

display_number=
xvfb_pid=
image_pid=
text_pid=
text_socket=
remote_cleanup_ready=true
cleanup() {
    if [[ -n "$image_pid" ]] && kill -0 "$image_pid" 2>/dev/null; then
        kill "$image_pid" 2>/dev/null || true
        wait "$image_pid" 2>/dev/null || true
    fi
    if [[ -n "$text_pid" ]] && kill -0 "$text_pid" 2>/dev/null; then
        kill "$text_pid" 2>/dev/null || true
        wait "$text_pid" 2>/dev/null || true
    fi
    if [[ -n "$xvfb_pid" ]]; then
        kill "$xvfb_pid" 2>/dev/null || true
        wait "$xvfb_pid" 2>/dev/null || true
    fi
    if [[ "$remote_cleanup_ready" == true ]]; then
        ssh "${ssh_options[@]}" "$ssh_target" rm -rf -- "$remote_dir" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

remote_image="$remote_dir/fixture.png"
remote_text="$remote_dir/fixture.txt"
remote_runner="$remote_dir/run-preview-and-restore.sh"
remote_image_started="$remote_dir/image-started"
remote_image_restored="$remote_dir/image-restored"
remote_text_started="$remote_dir/text-started"
remote_text_restored="$remote_dir/text-restored"

scp -q "${ssh_options[@]}" \
    "$runner" "$image_fixture" "$text_fixture" \
    "$ssh_target:$remote_dir/"

remote_version=$(ssh "${ssh_options[@]}" "$ssh_target" "$remote_bin" --version)
remote_version=${remote_version//$'\r'/}

remote_file_exists() {
    local path=$1
    ssh "${ssh_options[@]}" "$ssh_target" test -s "$path" >/dev/null 2>&1
}

wait_for_remote_file() {
    local path=$1
    local attempts=${2:-120}
    local attempt
    for ((attempt = 0; attempt < attempts; attempt += 1)); do
        remote_file_exists "$path" && return 0
        sleep 0.1
    done
    return 1
}

for candidate in $(seq 210 239); do
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
for _ in $(seq 1 100); do
    if env -u WAYLAND_DISPLAY DISPLAY=":$display_number" xdpyinfo >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done

image_title="terminal-interop-ssh-image-$RANDOM-$$"
remote_image_command="env TERM_INTEROP_BIN=$remote_bin TERM_INTEROP_FIXTURE=$remote_image TERM_INTEROP_RESTORED=$remote_image_restored TERM_INTEROP_STARTED=$remote_image_started TERM_INTEROP_DURATION_MS=12000 TERM_INTEROP_BACKEND=sixel bash $remote_runner"
env -u WAYLAND_DISPLAY \
    DISPLAY=":$display_number" \
    "$alacritty_bin" --config-file /dev/null \
        -o window.dimensions.columns=100 \
        -o window.dimensions.lines=32 \
        -o window.dynamic_title=false \
        -T "$image_title" \
        -e ssh "${ssh_options[@]}" -tt "$ssh_target" "$remote_image_command" \
        >"$run_dir/image.stdout" 2>"$run_dir/image.stderr" &
image_pid=$!

image_window=
for _ in $(seq 1 240); do
    image_window=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
        xdotool search --name "$image_title" 2>/dev/null | head -1 || true)
    [[ -n "$image_window" ]] && remote_file_exists "$remote_image_started" && break
    sleep 0.05
done
if [[ -z "$image_window" ]] || ! remote_file_exists "$remote_image_started"; then
    printf 'SSH image consumer did not become ready\n' >&2
    exit 1
fi
sleep 1.2

image_windows_before=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    xdotool search --name "$image_title" 2>/dev/null | wc -l)
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    import -window "$image_window" "$run_dir/image-live.png"
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    xdotool windowfocus "$image_window" key --window "$image_window" Return
sleep 0.4
if remote_file_exists "$remote_image_restored"; then
    printf 'Enter closed the SSH image preview\n' >&2
    exit 1
fi
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    import -window "$image_window" "$run_dir/image-after-enter.png"
image_windows_during=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    xdotool search --name "$image_title" 2>/dev/null | wc -l)
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    xdotool key --window "$image_window" q
if ! wait_for_remote_file "$remote_image_restored"; then
    printf 'q did not restore the SSH image caller\n' >&2
    exit 1
fi
sleep 0.2
env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    import -window "$image_window" "$run_dir/image-restored.png"
image_windows_after=$(env -u WAYLAND_DISPLAY DISPLAY=":$display_number" \
    xdotool search --name "$image_title" 2>/dev/null | wc -l)
wait "$image_pid"
image_pid=

if [[ "$image_windows_before" != 1 || "$image_windows_during" != 1 || "$image_windows_after" != 1 ]]; then
    printf 'SSH image preview opened or lost a separate window: %s/%s/%s\n' \
        "$image_windows_before" "$image_windows_during" "$image_windows_after" >&2
    exit 1
fi

image_geometry=$(identify -format '%wx%h' "$run_dir/image-live.png")
image_width=${image_geometry%x*}
image_height=${image_geometry#*x}
expected_height=$((image_width * 640 / 960))
magick "$image_fixture" -resize "${image_width}x${expected_height}!" "$run_dir/image-expected.png"
best_rmse=1
best_y=-1
for crop_y in $(seq 0 50); do
    if ((crop_y + expected_height > image_height)); then
        break
    fi
    magick "$run_dir/image-live.png" \
        -crop "${image_width}x${expected_height}+0+${crop_y}" +repage "$run_dir/image-candidate.png"
    metric=$(compare -metric RMSE \
        "$run_dir/image-expected.png" "$run_dir/image-candidate.png" null: 2>&1 || true)
    rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$metric")
    if [[ -n "$rmse" ]] && awk -v candidate="$rmse" -v best="$best_rmse" \
        'BEGIN { exit !(candidate < best) }'; then
        best_rmse=$rmse
        best_y=$crop_y
    fi
done
if ((best_y < 0)) || ! awk -v value="$best_rmse" 'BEGIN { exit !(value < 0.12) }'; then
    printf 'SSH Sixel framebuffer does not match the fixture: RMSE=%s y=%s\n' \
        "$best_rmse" "$best_y" >&2
    exit 1
fi
magick "$run_dir/image-live.png" \
    -crop "${image_width}x${expected_height}+0+${best_y}" +repage "$run_dir/image-live-crop.png"
magick "$run_dir/image-after-enter.png" \
    -crop "${image_width}x${expected_height}+0+${best_y}" +repage "$run_dir/image-after-enter-crop.png"
magick "$run_dir/image-restored.png" \
    -crop "${image_width}x${expected_height}+0+${best_y}" +repage "$run_dir/image-restored-crop.png"
enter_metric=$(compare -metric RMSE \
    "$run_dir/image-live-crop.png" "$run_dir/image-after-enter-crop.png" null: 2>&1 || true)
image_enter_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$enter_metric")
restore_metric=$(compare -metric RMSE \
    "$run_dir/image-expected.png" "$run_dir/image-restored-crop.png" null: 2>&1 || true)
image_restore_rmse=$(sed -n 's/.*(\([^)]*\)).*/\1/p' <<<"$restore_metric")
image_colors=$(identify -format '%k' "$run_dir/image-live.png")
if ((image_colors < 500)); then
    printf 'SSH Sixel preview retained only %s colors\n' "$image_colors" >&2
    exit 1
fi
if [[ -z "$image_enter_rmse" ]] \
    || ! awk -v value="$image_enter_rmse" 'BEGIN { exit !(value < 0.01) }'; then
    printf 'Enter altered the SSH image framebuffer: RMSE=%s\n' \
        "${image_enter_rmse:-unknown}" >&2
    exit 1
fi
if [[ -z "$image_restore_rmse" ]] \
    || ! awk -v value="$image_restore_rmse" 'BEGIN { exit !(value > 0.20) }'; then
    printf 'SSH image caller did not restore: RMSE=%s\n' \
        "${image_restore_rmse:-unknown}" >&2
    exit 1
fi

text_socket="$run_dir/kitty.sock"
remote_text_command="env TERM_INTEROP_BIN=$remote_bin TERM_INTEROP_FIXTURE=$remote_text TERM_INTEROP_RESTORED=$remote_text_restored TERM_INTEROP_STARTED=$remote_text_started TERM_INTEROP_DURATION_MS=12000 TERM_INTEROP_BACKEND=auto bash $remote_runner"
env \
    kitty --config NONE \
        --override allow_remote_control=socket-only \
        --listen-on "unix:$text_socket" \
        --start-as hidden \
        -- ssh "${ssh_options[@]}" -tt "$ssh_target" "$remote_text_command" \
        >"$run_dir/text.stdout" 2>"$run_dir/text.stderr" &
text_pid=$!

for _ in $(seq 1 180); do
    [[ -S "$text_socket" ]] && remote_file_exists "$remote_text_started" && break
    sleep 0.05
done
if [[ ! -S "$text_socket" ]] || ! remote_file_exists "$remote_text_started"; then
    printf 'SSH text consumer did not become ready\n' >&2
    exit 1
fi
for _ in $(seq 1 180); do
    if kitty @ --to "unix:$text_socket" get-text --match all \
        >"$run_dir/text-live.txt" 2>/dev/null \
        && rg -q --fixed-strings 'SSH TEXT PREVIEW SAFE' "$run_dir/text-live.txt"; then
        break
    fi
    sleep 0.05
done
rg -q --fixed-strings 'SSH TEXT PREVIEW SAFE' "$run_dir/text-live.txt"
rg -q --fixed-strings '␛[31mNOT_RED' "$run_dir/text-live.txt"
rg -q --fixed-strings 'Привет 世界' "$run_dir/text-live.txt"

kitty @ --to "unix:$text_socket" send-text --match all '\r'
sleep 0.4
if remote_file_exists "$remote_text_restored"; then
    printf 'Enter closed the SSH text preview\n' >&2
    exit 1
fi
kitty @ --to "unix:$text_socket" get-text --match all >"$run_dir/text-after-enter.txt"
rg -q --fixed-strings 'SSH TEXT PREVIEW SAFE' "$run_dir/text-after-enter.txt"
kitty @ --to "unix:$text_socket" send-text --match all 'q'
if ! wait_for_remote_file "$remote_text_restored"; then
    printf 'q did not restore the SSH text caller\n' >&2
    exit 1
fi
kitty @ --to "unix:$text_socket" get-text --match all >"$run_dir/text-restored.txt"
rg -q --fixed-strings 'TERM_INTEROP_RESTORED' "$run_dir/text-restored.txt"
wait "$text_pid"
text_pid=

ssh_version=$(ssh -V 2>&1)
jq -n \
    --arg schema 'urn:terminal-interop:ssh-preview-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg ssh_version "$ssh_version" \
    --arg remote_term_interop "$remote_version" \
    --arg alacritty_version "$("$alacritty_bin" --version)" \
    --arg kitty_version "$(kitty --version)" \
    --arg geometry "$image_geometry" \
    --argjson colors "$image_colors" \
    --argjson image_rmse "$best_rmse" \
    --argjson enter_rmse "$image_enter_rmse" \
    --argjson restore_rmse "$image_restore_rmse" \
    --argjson crop_y "$best_y" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        transport: {
            adapter: "openssh-pty",
            client: $ssh_version,
            remote_consumer: $remote_term_interop
        },
        image: {
            terminal: $alacritty_version,
            renderer: "sixel",
            geometry: $geometry,
            unique_colors: $colors,
            crop_y: $crop_y,
            normalized_rmse_to_fixture: $image_rmse,
            normalized_rmse_after_enter: $enter_rmse,
            restored_rmse_to_fixture: $restore_rmse,
            enter_does_not_close: true,
            q_restores: true,
            separate_window_never_created: true
        },
        text: {
            terminal: $kitty_version,
            text_visible: true,
            escape_rendered_as_text: true,
            unicode_visible: true,
            enter_does_not_close: true,
            q_restores: true
        }
    }' | tee "$run_dir/summary.json"

printf 'SSH_E2E_ARTIFACTS=%s\n' "$run_dir"
