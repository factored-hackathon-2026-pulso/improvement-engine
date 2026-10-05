#!/usr/bin/env sh
# Run the bridge from any directory: cd to the repo root of this script, pass all args through.
cd "$(dirname "$0")/../.." || exit 1
exec python scripts/o11y/runtrace_bridge.py "$@"
