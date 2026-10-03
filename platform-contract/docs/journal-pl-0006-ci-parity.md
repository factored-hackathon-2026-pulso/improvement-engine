# PL-0006: CI parity run

Branch claude/pl-platform rebased onto origin/main 4a1fa7b (only conflict: append-only docs/BITACORA_PULSO.md, kept both sides).
The only workflow, rust-ci, runs Rust jobs (untouched by this branch) plus the root contract-fixture harness
(`python -m unittest discover -s tests -p "test_*_contract.py"` and `contracts/validate_fixtures.py`), both green.

Additional checks for the new directories (all green, no fixes needed):
- platform-contract pytest: 26 passed; contract drift check clean (regenerated schemas only differ by CRLF on Windows).
- platform-sim tests/plive: 20 passed (needs sqlglot); platform-sim full suite excluding plive: 206 passed.
- platform-exporter: 51 passed against a throwaway postgres:16 (Podman pulso-dev, removed afterwards).
- ruff --isolated --select E4,E7,E9,F over the new Python dirs: clean.
- debug-console (Node 22): npm ci, typecheck, vitest 121 passed, build ok.

Operational note: stray `vite preview` processes from earlier work held esbuild.exe and made `npm ci` fail with EPERM.
