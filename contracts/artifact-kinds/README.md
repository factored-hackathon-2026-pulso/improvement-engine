# artifact-kinds (BK0, frozen artifact)

`matrix.json` is the artifact-kind capability matrix: for every agent-core kind
(11 `EntityKind` values, the draft-only kind `eval_suite`, and `release_settings`)
it gives the four verbs `propose`, `validate`, `evaluate`, `publish` a verdict
(`supported`, `denied(reason, denied_by)`, `not_exercised`, `blocked(blocked_on)`),
an evidence level (`real-core-live`, `real-core-dry-run`, `stand-in`, `none`) and
evidence pointers (`path::test_name`, checked to exist). It also lists the
supported and denied change families and the denied kinds with who denies them
(`bridge` = core-bridge guardrail, `engine` = our compile step).

Files: `matrix.schema.json` (JSON Schema 2020-12), `DIGEST.json` (sha256 of the
canonical matrix, the published frozen artifact; `python kinds_digest.py --write`
refreshes it), `tests/test_matrix.py`.
Run: `python -m pytest contracts/artifact-kinds/tests` (needs `jsonschema`; the
drift test reads the pinned checkout `AGENT_CORE_REF`, default
`D:/.codex/factored/references/agent-core-c814c2b`, and skips if absent).

## What is true today (honest summary)

- Real on the real Core: only `prompt` (op replace) and `eval_suite` (op add),
  all four verbs, evidence `e2e-core/THREAD01.md` steps 5, 6, 8, 9.
- Everything else: propose is `denied(kind_not_supported)` by the engine compile
  step; validate/evaluate/publish are `not_exercised` (the Core dry-run is
  kind-generic, but only `flow` has a dry-run test, as a violation case).
- `tool`: validate/evaluate/publish `blocked(executor-registration)` (BKOL).
- `decision_model`: provider `jev` is `blocked(jev)`.
- `release_settings`: `denied(release_settings_not_allowed)` by the bridge.

## How Team Codex consumes it (offline work)

1. Pin `DIGEST.json` `digest`; a changed matrix is a `[CONTRACT-CHANGE]`.
2. BKF (flow + compiled tree), BKT (template, policy, prompt-variant), BKD, BKA,
   BKJ: your kind module turns the `propose` row of that kind from
   `denied(kind_not_supported)` into a golden-backed offline behaviour. The matrix
   says what is verifiable against the real Core today (nothing for these kinds
   except `flow` dry-run validation): offline goldens are yours, mark them
   `stand-in`, never `real-core-*`. Claude's live WPs (BKFL, BKTL, BKDL, BKAL)
   raise evidence levels and update the matrix.
3. `prompt-variant` is `prompt` with op add (`add_prompt`, denied today); there is
   no separate Core kind. Compiled trees are `flow` content.
4. Authorized-field limits are NOT defined by the pinned Core or this matrix;
   BKT/BKA define them and the matrix only records whether a kind is proposable.
5. Denied negatives (BKN) use the named reasons in `reasons`: `tool_without_executor`,
   `unsupported_capability`, `release_level_change` are engine-side names reserved
   for BKN; `blocked(jev)` is a verdict, not a reason.
6. Denial by `bridge` (release_settings) cannot be lifted by the engine.
