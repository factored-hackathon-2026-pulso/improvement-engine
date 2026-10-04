# W0 status (Team CL, PR #94 `claude/w0-train`)

Placed under `docs/agents/**` (L-GOV) because `docs/claude/**` has no owner in `OWNERS.md`.
Evidence labels: **real**, **real-narrow**, **stand-in**, **agent_roleplay** (role-played, no quality claim),
**simulated**, **recorded**, **blocked**. Nothing here is pushed or merged beyond what PR #94 states.

## What is built (work-package ids)

| Group | Ids | Where |
|---|---|---|
| Governance | G0f (owner map), CONTR (schemas, ports), FRZ0 (digest-pinned pack), G0gp/G0gr (public-surface audit, PR #93 review), G1 (engine-run honesty checks), G0p | `OWNERS.md`, `docs/agents`, `contracts/engine-steps`, `contracts/engine-run`, `docs/reports/gates`, `scripts/gov` |
| Env | G0e (target dirs, one cargo slot), WTR1 (inventory) | `scripts/env` |
| Model | TPS (treated-payload scanner), M0RP/RPP (roleplay shim and protocol), DC0 (data-class gate), M1 (gateway overlay), M2a (spend guard), M3 (stage caps, schema subset, timeouts) | `roleplay-llm`, `scripts/dc`, `local/core/gateway`, `core-bridge/.../llm`, `.../stages` |
| Bridge / facts | CLT0 (invoke timeout then reconcile), SNET (partial), WRLD0 (seeded base world), SMAP-lite corpus | `core-bridge`, `agent-core-assets/{worlds,corpus,tools}` |
| Platform | P2py (release events, fast-forward clock, author-separated effects) | `platform-sim/platform_live` |
| E2E stand-ins | ED0 (Rust sensor detection), ED0L (k-anonymous lab), SMAP, CMPpy, GSIpy, Q1r (ten-step ratchet), INT0 hooks (step 5 dry run, alias read) | `e2e-core/src/claude_standin` |

## Test commands (verified from the repo root, uv, no cargo, no containers)

    uv run --python 3.12 python -m unittest discover -s roleplay-llm/tests   # run from roleplay-llm/ (cd first)
    uv run --python 3.12 --with pytest --with pyyaml python -m pytest contracts/engine-run/tests scripts/dc/tests scripts/gov/tests scripts/env/tests local/core/gateway/tests docs/agents/tests -q -p no:cacheprovider
    cd agent-core-assets && uv run --python 3.12 --with pyyaml --with pytest python -m pytest -c pytest.ini tests/test_world_seeded_base.py -q -p no:cacheprovider
    cd e2e-core && PYTHONPATH='src;tests;../core-bridge/src;../local-identity/src;../platform-sim;../platform-contract;..' \
      uv run --python 3.12 --with pytest --with pyyaml --with fastapi --with httpx --with cryptography \
      python -m pytest tests/unit/test_e2e_thread_01.py -q -p no:cacheprovider

`docs/agents/tests/test_owners.py` was red because 54 `.nexus-outbox/*.json` receipts had been committed; they are now
untracked and ignored (`/.nexus-outbox/` in `.gitignore`). core-bridge, platform-sim and the live e2e tests need the
pinned venv, Postgres or Podman and were not run for this document; see their READMEs.

## Ten-step thread in replay (`e2e-core/THREAD01.md`)

`claude_standin/thread01.py` runs steps 1-10 on the Python host: the Rust sensor exe (`ED0_RUNNER_EXE`, step 2 reports
`blocked(sensor-exe)` if absent), the roleplay shim in `--replay-only` over recorded synthetic responses
(`tests/fixtures/thread01_queue/responses`, re-record with `python -m claude_standin.thread01 --record <dir>`),
SMAP catalogue, CMPpy, GSIpy, a simulated issuer (JWS bound to the draft digest), a registry double, and platform-sim
release events. The final report must pass `contracts/engine-run` `check()`; `doubles[]` is generated, never hand-written.

## Stand-in or blocked

- Step 1 manual command (scheduled ingest is DEMO-2); step 3 and 4 are `agent_roleplay`; steps 5 (CMPpy) and 6
  (GSIpy, structural only) are stand-ins unless `CoreHooks` are supplied; step 7 `not_exercised` unless a gate fails (then steps 8-9 are `blocked(gate)` unless a labelled human override, G1 rule);
  step 8 simulated issuer; step 9 registry double; step 10 simulated, observation only.
- Real Core: step 5 dry run verified live once (PG16, pinned Core image). `run_arms` and `publish` need a frozen proposal
  plus a human JWS; Core does not yet verify the JWS (INT0).
- Real gateway: live smoke skipped until containers run; healthcheck path unverified. SNET container-side network
  measurements blocked (rootless cgroups). PG-recorded claim traces in FRZ0 still to come.
- CONTRACT-PUBLISHED for the pack is pending until it is on GitHub main (Codex asked for a source SHA and digests).

## Open risks (journal CL-0041..CL-0043 and reviewers)

1. TPS cannot detect opaque ids (a name or national id shaped like an allowed token passes); needs a registry of expected ids.
2. The ED0L lab has no complementary suppression.
3. The M2a spend guard is not wired into `main.py`/factories yet.
4. Core-side steps (dry run, evaluation, publish, alias read) stay stand-in until INT0.
5. `release.published`/`release.rolled_back` classify as `unknown` (quarantined) until Codex admits them in event-catalog 1.1.0.
6. G0gr facts for Codex: `ArmBinding` fields are public (forgeable); `PlatformTreatedText::from_authoritative_treatment` always returns AuthorityUnavailable; the default read plan includes EventPayloadLocalOnly; `read_relation` returns `Result<(), E>`.
7. Machine load is high (about 4 GB free); no cargo or containers were run for this document.
