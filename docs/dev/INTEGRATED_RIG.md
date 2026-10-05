# Integrated rig, stage 1 (ENV1): engine + agent-core + llm-gateway + support-platform at API level

Own prefix `pulso-env1`; ports Postgres 55510, gateway 8210, agent-core 8211, engine 4210, platform 8200 (the LFC lane uses 55510/8210 too: do not run both).
Scripts (Windows PowerShell 5.1 and pwsh): `scripts/integrated-rig/up.ps1 | health.ps1 | run_story.ps1 | down.ps1`, `merge_keys.py`, `story_verify.py`.
Memory gate: `up.ps1` starts only above 1500 MB free RAM (`-WaitRamMin`, `-Force`); `-ReuseStack` keeps a running Postgres/gateway/agent-core.
Credentials: `agent-core.env` / `llm-gateway.env` go only into child environments (via `scripts/demo-loop/run.ps1` and `Invoke-Scrubbed`); only variable NAMES are logged; the platform service token is generated per `up` and kept in the gitignored `.dev-stack/integrated-rig/secrets.json`. Pester canary tests prove no value is printed.

## Wiring (what is actually set)

| Hop | Wiring |
|---|---|
| engine -> gateway / agent-core | `PULSO_LLM_GATEWAY_KEY`; `PULSO_REGISTRY_TOKEN` = builder principal `pulso-engine`, role constructor, kid `pulso-engine-dev-1` |
| platform -> agent-core | `CC_AGENT_CORE_URL`, `CC_AGENT_KEYS_FILE` (`gen_agent_keys`, fresh suffix per up) |
| announce adopt (G6) | `AgentBuilder.announce_proposal` signs, with the platform STAFF key, a `builder` credential for staff id `engine`, role constructor only (no approver, no step-up), and reads the proposal from agent-core `/v1/registry`; it must be `origin=auto_detect`. agent-core accepts it because the merged `staff-keys.json` holds the platform staff kid. Live: announce 200, replay 200 |
| engine/rig -> platform | `POST /api/v1/internal/builder/proposals/announce`, bearer `CC_INTERNAL_SERVICE_TOKEN`; evidence ids from `GET /api/v1/internal/evidence/cases` (same token) |
| agent-core -> platform | `AGENTCORE_GRANTS_URL`, `AGENTCORE_GRANTS_TOKEN` (not used by the builder flow) |
| poller -> agent-core export | `exporter` token now minted by `scripts/dev-stack/identity.py mint` (G9, read-only, engine kid) |
| key merge (G10) | `merge_keys.py`: union of dev-stack keys (dev admin kid, engine kid) and platform keys into `identity-keys.json` and `staff-keys.json`; same kid with another key is refused; prints kids only |

## Last verified run (2026-10-05, this machine, planted profile)

Free RAM 2.3 GB at start. Models: Scout/Verifier/Builder via the gateway (Builder `xiaomi/mimo-v2.6-pro` requested, `flash` fallback; loop cost USD 0.0019). Aggregate transcript:

```
[OK] cells: synthetic-planted: 350 rows, metrics M4=350            (SYNTHETIC, invented numbers)
[OK] evidence: 8 CASE- ids from the platform evidence route (matched 17 seeded cases)
[OK] loop: 262 s; corroborated 1, proposed 1, delivered 1, announced 1
     finding_1 M4 pqr_open_rate -> template:t/estado_pqr, regression_suite_proven (the base fails 8 of 8 cases, the candidate passes)
[OK] announce: HTTP 200, replay HTTP 200, 2 evidence links
[OK] verify: 22/22 checks
     agent-core: origin auto_detect, state draft, created_by pulso-engine, dossier on 2/2 changes, eval_suite + template changes
     one improvement_proposed notification each for Lucia Herrera, Martin Salazar, Renata Villalba, Felipe Echeverri
     listed in Automatizacion with source engine; detail title, docs.description, docs.rationale, docs.changelog present
[OK] eval: pulso-min suite attached by script on agent consultas; evaluate passed every gate; approve/publish/promote never called
[not-run] approve, publish, promote, release event -> poller -> engine, outcome step
```

Real: Postgres, gateway, agent-core serve, models, engine loop and proof, platform API/DB/seed, announce, notifications, list, detail, key trust. Stand-in: planted cells (synthetic), agent-core e2e tool doubles, seeded login (`demo1234`, dev MFA code, `CC_ENV=dev`), pulso-min eval suite (R6). Relaxed: the announce POST is sent by the rig from the engine record because the engine builds only opaque `CASE-` ids (G1); the ids are real seeded platform cases labelled as examples.

## Not run, and why

Approve, publish, promote (dev step-up), the release event through the poller and the outcome step were NOT exercised: the permission layer denied the automation that would approve/publish/promote through the platform, so the hops are reported as not run and need a user decision (the routes are `POST /builder/proposals/{id}/approve|publish`, `POST /builder/aliases/{agent}/prod/promote`, body `stepUpCode`, see `support-platform` `api/routers/builder.py`). Outcome additionally needs `PULSO_OUTCOME_*` (pseudo release, planted post cells, `scripts/out1/outcome_cli_adapter.py`) and a persistent engine process, because trigger records are in memory.

## Gaps found

| # | Gap | Owner |
|---|---|---|
| E1 | `uvicorn`/`cc-api` on Windows fails: `ZoneInfo("America/Bogota")` needs `tzdata` (not a dependency). The rig runs `uv run --with tzdata` | platform: add `tzdata` to backend deps (marker `sys_platform == 'win32'`) |
| E2 | `GET /api/v1/meta` no longer reports `agentCoreConfigured`; the rig checks builder availability through a supervisor session instead | platform (doc) |
| E3 | Engine announce builds opaque `CASE-` ids; use `GET /internal/evidence/cases` (exists since platform PR 27) in `registry_writer::announce` | engine (L-ENGINE) |
| E4 | `PULSO_PROFILE=demo` floors (R4) are not implemented in the engine; the planted effect passes without them | engine |
| E5 | Trigger records live in memory: an outcome needs the same engine process from loop to release event | engine |
| E6 | Ports 55510/8210 are also used by the LFC lane | process |
| E7 | `New-AnnouncePayload` returned a string for a single evidence link (`$(...)` unrolls arrays); fixed here with a regression test | engine (done) |
