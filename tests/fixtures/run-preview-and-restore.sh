#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_BIN:?TERM_INTEROP_BIN is required}"
: "${TERM_INTEROP_FIXTURE:?TERM_INTEROP_FIXTURE is required}"
: "${TERM_INTEROP_RESTORED:?TERM_INTEROP_RESTORED is required}"

backend=${TERM_INTEROP_BACKEND:-kgp}
duration_ms=${TERM_INTEROP_DURATION_MS:-2500}

if [[ -n "${TERM_INTEROP_STARTED:-}" ]]; then
    printf 'started\n' >"$TERM_INTEROP_STARTED"
fi
if [[ -n "${TERM_INTEROP_START_FIFO:-}" ]]; then
    IFS= read -r _ <"$TERM_INTEROP_START_FIFO"
fi

"$TERM_INTEROP_BIN" preview "$TERM_INTEROP_FIXTURE" \
    --backend "$backend" \
    --exit-after-ms "$duration_ms"

printf '\033[2J\033[HTERM_INTEROP_RESTORED\n'
printf 'restored\n' >"$TERM_INTEROP_RESTORED"
sleep 3
