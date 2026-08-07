#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_BIN:?TERM_INTEROP_BIN is required}"
: "${TERM_INTEROP_RECEIPT:?TERM_INTEROP_RECEIPT is required}"
: "${TERM_INTEROP_STARTED:?TERM_INTEROP_STARTED is required}"
: "${TERM_INTEROP_START_FIFO:?TERM_INTEROP_START_FIFO is required}"

printf 'ready\n' >"$TERM_INTEROP_STARTED"
read -r _ <"$TERM_INTEROP_START_FIFO"

"$TERM_INTEROP_BIN" negotiate pixel \
    --timeout-ms "${TERM_INTEROP_TIMEOUT_MS:-2000}" \
    --output "$TERM_INTEROP_RECEIPT" \
    >"${TERM_INTEROP_NEGOTIATION_LOG:-/dev/null}" 2>&1
