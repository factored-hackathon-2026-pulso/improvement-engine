#!/usr/bin/env bash
# Offline + agent-core validation of agent-core-assets. Requires uv; AGENT_CORE_CHECKOUT optional.
set -euo pipefail
cd "$(dirname "$0")/.."
uv run --python 3.12 --with pyyaml python tools/assetcheck.py check
uv run --python 3.12 --with pyyaml python tools/assetcheck.py validate
