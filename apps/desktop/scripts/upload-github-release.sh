#!/usr/bin/env bash
# Thin wrapper — prefer: npm run publish:github
exec node "$(dirname "$0")/upload-github-release.mjs" "$@"
