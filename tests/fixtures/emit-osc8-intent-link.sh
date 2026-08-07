#!/usr/bin/env bash
set -euo pipefail

: "${TERM_INTEROP_TEST_URI:?TERM_INTEROP_TEST_URI is required}"

printf '\033]8;;%s\033\\test-photo.png\033]8;;\033\\\n' "$TERM_INTEROP_TEST_URI"
printf 'OSC8_INTENT_LINK_READY\n'
sleep 30
