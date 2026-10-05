# Engine on the production host: serve credentials, the loop job, the demo profile (ENGPROD)

What the engine host runs besides `pulso run`, and the environment contract infra wires. Names only: no value appears here, in a log or in a record.
Related: `docs/dev/ENGINE_IMAGE.md` (the image), `docs/dev/INTEGRATED_RIG.md` (the local rig), infra `deploy/hackathon/engine/compose.loop.yaml` (the two engine-code slots this closes).

## 1. Credentials for agent-core `serve` (minted, short-lived)

`serve` verifies the registry API (`/v1/registry/*`) against the public keys in its `--staff-keys` file and refuses a credential whose `exp` has passed.
It does not cap the lifetime, so the issuer chooses it. The engine is that issuer for its own service identity:

| Item | Value |
|---|---|
| Format | compact JWS, header exactly `{alg: EdDSA, kid, typ: principal+jws}`; payload a `Principal` |
| Claims | `type=builder`, `id=pulso-engine`, `roles=["constructor"]`, `scopes=[]`, `attrs={}`, `auth={level: session, at}`, `exp` |
| Key | Ed25519; the 32-byte seed is `PULSO_SERVICE_SEED_HEX` (64 hex); the public key is listed under `PULSO_SERVICE_KID` in the Core's staff-keys |
| Lifetime | 300 s by default (`PULSO_SERVICE_CRED_TTL_S`, 60..900); a credential is replaced when a fifth of its life is left |
| Where | `core_client::service_identity::ServiceIdentity` (cache + fake-clock tests); `registry_writer::MintingTransport` puts a fresh credential on EVERY registry request and, after a 401, mints once more and repeats the request once |
| Never | an approver role, a human attribute, a step-up level. `engine_never_approves_publishes_or_promotes` stays true |

Selection, in `pulso::run::registry_auth`: `PULSO_REGISTRY_AUTH=static|mint`. Unset: `static` when `PULSO_REGISTRY_TOKEN` is set (the local stack and the rig, unchanged), else `mint` when the seed is set, else a refusal naming both. `mint` needs `PULSO_REGISTRY_VIA=api` and refuses `PULSO_REGISTRY_CREDENTIAL=standin`. The delivery label says `engine builder principal (minted from the service seed, short-lived)`.

The old core-bridge path (`PULSO_CORE_PORT=live`, `PULSO_BRIDGE_ADDR`, `PULSO_HUMAN_*`) is untouched and still behind its switch; `pulso loop` does not read it.

Trust (infra ADR 0009, accepted): every key in staff-keys can claim any role. The engine never signs an approver role; that is code discipline, not a Core ceiling.

Verified: unit tests with a fake clock; a contract test against the real agent-core verifier and registry HTTP extension (`scripts/serve-credentials/test_serve_credential_contract.py`, section 5); a live run against a local `agentcore serve` (journal 0663).

## 2. The loop job: `pulso loop`

One run of the improvement loop as a process of its own (no HTTP service, no job store): cells -> sensor -> Scout, Verifier, Builder, deterministic recompute, compile -> regression proof on agent-core -> registry writer as the builder principal -> announce to the platform. It is the same `ValueLoop` that `pulso run` drives for a `trigger:*` job. Run it from a systemd timer or `docker compose ... run --rm pulso-loop`; infra sets the command to `pulso loop` (`PULSO_LOOP_COMMAND=loop`, entrypoint `pulso`).

`pulso loop --check` validates the configuration and prints one JSON line (`loop_check`) without running anything or taking the lock. `--break-lock` removes a lock left by a killed run, then runs.

### Environment contract (names; `R` required, `O` optional)

| Name | | Meaning |
|---|---|---|
| `PULSO_LOOP_INPUTS_DIR` | R* | directory of the S3-synced inputs mirror (`engine/inputs`); the cells package is `PULSO_LOOP_CELLS_FILE` inside it (default `cells.ndjson`) |
| `PULSO_CELLS_NDJSON` | R* | explicit path of the cells package; wins over the two above (`*` one of the two is required) |
| `PULSO_CELLS_SOURCE` | R | `synthetic`, `bank` or `e0`: what the cells are. `PULSO_PROFILE=demo` only with `synthetic` |
| `PULSO_WORK_DIR` | R | writable: `loop.lock`, `loop-store/` (per-finding records), `registry-receipts.json`, `w11-proofs.json`, `loop-runs/<run>.json`, `loop-latest.json` |
| `PULSO_REGISTRY_ADDR` | R | `host:port` of agent-core `serve` (registry); falls back to `PULSO_CORE_ADDR`, which infra already renders (core IP:8001, plain HTTP, IP literal) |
| `PULSO_SERVICE_SEED_HEX`, `PULSO_SERVICE_KID` | R | the engine service identity (section 1); already rendered into `pulso.env` |
| `PULSO_REGISTRY_AUTH` | O | `static` or `mint` (default described in section 1) |
| `PULSO_REGISTRY_TOKEN` | O | static builder credential (local stack only) |
| `PULSO_REGISTRY_ENV` | O | `local` (default) or `shared`: the label of where proposals go. Infra sets `shared` |
| `PULSO_REGISTRY_VIA` | O | `api` (default) or `run` (not with minted credentials) |
| `PULSO_SERVICE_PRINCIPAL_ID`, `PULSO_SERVICE_CRED_TTL_S` | O | principal id (default `pulso-engine`), credential lifetime (60..900 s) |
| `PULSO_LLM_GATEWAY`, `PULSO_LLM_GATEWAY_ADDR`, `PULSO_LLM_GATEWAY_KEY` (+ `_MODEL`, `_VERIFIER_MODEL`, `_BUILDER_MODEL`, `_BUILDER_ESCALATION_MODEL`, `_ALIAS`, `_MAX_TOKENS`, `_TIMEOUT_S`, `_STRUCTURED`) | R | the model ports: the loop has no scripted configuration. `PULSO_LLM_GATEWAY=enabled` |
| `PULSO_EVAL_BEFORE_ANNOUNCE` | O | `on` (default for `via=api`) or `off`; `on` needs Python (see section 4) |
| `PULSO_REGRESSION_PYTHON`, `PULSO_REGRESSION_SCRIPTS`, `PULSO_EVAL_TIMEOUT_SECS` | O | proof runtime (default `python`, `scripts/regression`, 900 s) |
| `PULSO_ANNOUNCE_TO_PLATFORM`, `PULSO_PLATFORM_URL`, `PULSO_PLATFORM_SERVICE_TOKEN` | O | announce to the support platform (on only when both URL and token are set) |
| `PULSO_PROFILE` | O | `standard` (default) or `demo` (section 3) |
| `PULSO_LOOP_MAX_FINDINGS`, `PULSO_LOOP_MAX_EXPLORATORY` | O | cost bounds per run (exploratory default 2; 5 under `demo`) |
| `PULSO_ALLOW_DERIVED_AGGREGATES`, `PULSO_NEW_AGENT_ADMIN` | O | opt-ins of the value loop (default off) |
| `PULSO_LOOP_LOCK_TTL_S` | O | a lock older than this is taken over (default 7200, minimum 60) |
| `PULSO_LOOP_RUN_ID` | O | run id (default `loop-<unix seconds>`); the debug-api run id is `value-loop-<id>` |

`PULSO_CORE_PORT`, `PULSO_MODEL_PORT`, `PULSO_CORE_URL`, `STEPS_RUNNER_EXE` set in `compose.loop.yaml` are not read by `pulso loop` (harmless): the sensor is a library call, not `steps_cli`.

### Exit codes (systemd)

| Code | Meaning | Unit hint |
|---|---|---|
| 0 | the run finished; every finding has a closed outcome (blocked, denied, unlinked included) | success |
| 1 | the run could not run: unreadable or missing cells (inputs not synced), sensor error, model setup, work dir, results not written | failure, timer retries |
| 2 | refused configuration (a named variable, never a value) | `RestartPreventExitStatus=2`: do not retry as is |
| 3 | finished, but at least one finding ended on an infrastructure failure (registry unreachable or unauthorized, evaluation `failed_infra`, proof suite error, model unavailable); the records are kept and a re-run resumes them | failure, alert |
| 75 | another run holds `loop.lock`; nothing was done | `SuccessExitStatus=75` |
| 143 | SIGTERM or SIGINT during a run: `loop.lock` is released first, finished findings stay in `loop-store/`, a re-run resumes | `SuccessExitStatus=143` for a stop |

### Run lock, results, logs

- One lock, `<PULSO_WORK_DIR>/loop.lock`, created exclusively (`create_new`), released when the run ends. A run killed with SIGKILL leaves it; the next run takes it over after `PULSO_LOOP_LOCK_TTL_S` (or immediately with `--break-lock`). Put the TTL above the longest expected run (the proof takes minutes per finding).
- Per-finding records in `loop-store/`; a re-run over the same findings reuses them (no second model call; the writer's receipt store never opens a second proposal for the same finding).
- Results: `loop-runs/<run>.json` and `loop-latest.json` (ids, reason codes and numbers only: no token, no model free text).
- Logs: one JSON object per line on stdout, through the engine logger (it redacts secret-looking keys). Events: `loop_start`, `loop_finding` (one per finding), `loop_done` (summary, `infra_failures`), `loop_failed`, `loop_refused`, `loop_locked`, `loop_check`. Names only in `auth_mode`, `support_profile`, `source`.
- SIGTERM/SIGINT are trapped (`guard_termination`): the lock is released, `loop_terminated` is logged, exit 143. Only SIGKILL leaves the lock (TTL above).

## 3. `PULSO_PROFILE=demo` (R4): lower support floors, nothing else

| | standard | demo |
|---|---|---|
| strict tier pooled support (`min_support`) | 500 | 100 |
| exploratory tier pooled support | 200 | 60 |
| exploratory findings per run | 2 | 5 (`PULSO_LOOP_MAX_EXPLORATORY` still wins) |
| `k_min` (privacy floor) | 10 | 10, always (`steps::cells::K_FLOOR`; `sensor_config` asserts it) |
| alpha, effect, ratio, multiplicity, replication | unchanged | unchanged |
| allowed data | any | only `PULSO_CELLS_SOURCE=synthetic`; otherwise the process refuses (exit 2) |

Never implicit: an unset variable is `standard`; an unknown value is refused. Every report carries `method.support_profile`, and the loop summary carries `support_profile`, so a dossier or a card can say which floors produced it. The floors live in one place (`steps::cells::Config::demo`).

## 4. Image

The runtime image has `pulso`, `pulso-synth-runner`, `steps_cli`, `python3` + `python3-yaml` and the regression proof (`/opt/pulso/scripts/regression/{build_suite,judge_story,prove_fails_on_base}.py`, `/opt/pulso/agent-core-assets/eval-suites/{pulso-min,pulso-w13}`), non-root (uid 10001). Defaults `PULSO_REGRESSION_PYTHON=python3`, `PULSO_REGRESSION_SCRIPTS=/opt/pulso/scripts/regression`, so `pulso loop` runs the proof and announce in it. The scripts import only the standard library and PyYAML.

## 5. Tests and how to run them

```text
cd seams
cargo test -j 1 -p core-client --lib service_identity
cargo test -j 1 -p registry-writer --lib minting
cargo test -j 1 -p steps --test cells demo
cargo test -j 1 -p pulso --lib
cargo test -j 1 -p pulso --test value_loop --test loop_process
python -m unittest docs/agents/tests/test_owners.py
# contract against the pinned agent-core verifier (needs an agent-core checkout and its Python deps):
cargo build -j 1 -p core-client --example mint_service_credential
$env:AGENT_CORE_DIR='<agent-core checkout>'; $env:PULSO_MINT_EXE='<target>\debug\examples\mint_service_credential.exe'
<python with agent-core deps> -m unittest scripts/serve-credentials/test_serve_credential_contract.py
```
