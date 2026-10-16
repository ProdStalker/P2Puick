#!/usr/bin/env bash
# Thin wrapper — prefer: npm run publish:update
exec node "$(dirname "$0")/publish-update.mjs" "$@"
