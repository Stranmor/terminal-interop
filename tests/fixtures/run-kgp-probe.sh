#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_BIN:?TERM_INTEROP_BIN is required}"
: "${TERM_INTEROP_RECEIPT:?TERM_INTEROP_RECEIPT is required}"
: "${TERM_INTEROP_PROBE_LOG:?TERM_INTEROP_PROBE_LOG is required}"
: "${TERM_INTEROP_STARTED:?TERM_INTEROP_STARTED is required}"
: "${TERM_INTEROP_START_FIFO:?TERM_INTEROP_START_FIFO is required}"

printf '%s\n' "$$" >"$TERM_INTEROP_STARTED"
IFS= read -r _start_signal <"$TERM_INTEROP_START_FIFO"

transport_args=()
if [[ -n "${TERM_INTEROP_TRANSPORT:-}" ]]; then
    transport_args=(--transport "$TERM_INTEROP_TRANSPORT")
fi

exec "$TERM_INTEROP_BIN" probe kgp \
    --timeout-ms "${TERM_INTEROP_TIMEOUT_MS:-1500}" \
    "${transport_args[@]}" \
    --output "$TERM_INTEROP_RECEIPT" \
    --pretty \
    2>"$TERM_INTEROP_PROBE_LOG"
