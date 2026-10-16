#!/usr/bin/env bash
# Thin wrapper — prefer: npm run update
exec node "$(dirname "$0")/release-update.mjs" "$@"
