# Dependency evidence fixtures (sanitized)

Two cases, produced from the REAL pinned Core (agent-core 789d6c8) on PG16 and the REAL exporter:

- `outage_unavailable.json`: an agent-node gateway outage. The audit chain contains one `agent_step` with `kind=failed`, `error_kind=unavailable`; the task receipt is the unchanged `terminal_failed` / `escalated` / `unexpected_outcome`.
- `low_confidence_gave_up.json`: a legitimate "could not conclude" (the model answered, output invalid twice). Same receipt, but NO `agent_step` with `kind=failed`.

Only the audit event identifies a dependency failure (agreement CX-0073/0075). Hashes, ids and timestamps are placeholders; no prompts, inputs, keys or payload text. `observation` is the exact key set of the exported `core_event` observation.

Regenerate (needs a throwaway PG16, see `tests/runtime/conftest.py`):

    PULSO_TEST_PG_ADMIN=postgresql://postgres:<pw>@127.0.0.1:<port>/postgres PULSO_REGEN_DEPENDENCY_EVIDENCE=1 python -m pytest tests/dependency_evidence -q

Without the env switch the same test only checks that the checked-in files keep the shape the real stack produces.
