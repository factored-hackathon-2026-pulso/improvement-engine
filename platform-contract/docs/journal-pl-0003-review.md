# Journal PL-0003: independent adversarial review of PL-L3 / PL-L2

Scope: commits b170e42, b591863 vs Product artifact a492bfa. Test: 37 -> 46 passing.

## Defects found and fixed (TDD, failing tests first)
- MEDIUM privacy: `validate_rows` echoed jsonschema messages that embed the offending value
  (e.g. a >500 char `close_note` was reproduced in full). Messages now name path + keyword only.
- MEDIUM vacuous: `format: date-time` was NOT enforced (jsonschema needs an optional rfc3339 lib);
  "not-a-date" validated. Added a UTC pattern in the generated schemas plus a registered format check.
- LOW: `turns.author_id` / `cases.closed_by_id` lacked CUS-/STF- prefix patterns.
- LOW fidelity: artifact says `client_message_id` is unique per author; DDL lacked it (partial unique index added).
- LOW: Postgres DDL had no append-only enforcement; added `append_only_guards_postgres()`.

## Verified
- Postgres DDL (+ guards, unique index) executed on a real postgres:16-alpine (pulso-dev, --cgroups=disabled):
  all statements OK, UPDATE/DELETE on event_log rejected. Container removed.
- Mutation checks (13 mutants over simulator rules, conformance, denylist, additionalProperties, guards): all killed.
- Drift test genuinely regenerates from model.py; `python -m platform_contract` rewrites files.
- Invalid golden examples fail for the intended reason.

## Unresolved / assumptions
- `assigned_by_role` enum includes `admin` (artifact says system or supervisor): assumption.
- Event payload shapes, planned `staff.*`/`team.*` handling remain assumptions (documented).
- Running pytest with trailing `.` collects unrelated suites (fastapi missing); use the two dirs.
