# Replay cohort state transitions

## Scope

Added a typed, in-memory transition model for the DREPLAY kernel. Frozen replay
keeps its initial revision across cohorts. Prequential replay accepts one
revision update only after the active cohort has been sealed with an opaque
evaluator receipt; that revision remains inactive for the sealed cohort and
becomes active only when a strictly later cohort begins. The receipt value is
stored only as a seal marker and is never passed to decision/update inputs.

Cohort leases bind a cohort ID, timestamp, sorted case membership, and active
revision. Exact retries of the currently active cohort return its same lease;
cohort IDs cannot be reused after closure. A checkpoint/restore preserves the
last timestamp, active lease/seal, pending/applied update IDs, and consumed
cohort/case membership. The state model rejects a case appearing in a later
cohort, including after checkpoint/restore, so one replay episode cannot be
scored or learned twice through this boundary.

This is not durable storage or the E0 runner/evaluator/CLI. It does not itself
persist checkpoints, read labels/timeline, execute a detector, evaluate a
proposal, or prove a full restart/recovery campaign. Evaluator integration and
the actual atomic persistence boundary remain separate work.

## TDD and review

Focused tests followed RED-GREEN loops. The initial frozen behavior failed at
the unimplemented state transition, then passed. The prequential transition
test failed at the missing update implementation, then passed after introducing
sealed-only pending updates with next-cohort activation. Independent adversarial
review then found a P2: case IDs were only deduplicated inside one cohort. A
checkpoint/restore regression failed because `case-a` could be admitted again
under a later cohort; the machine now tracks consumed case membership in its
checkpoint and rejects reuse. The final focused target passed 4/4 with pinned
Rust formatting applied. No full local CI, PostgreSQL, Podman, GitHub Actions,
commit, or publication was run for this slice.

Synthetic fixtures are derived from V3 §§28.4, 28.5 E0-C and 28.7.2. FRZ0 is
not claimed as provenance for temporal cohort fixtures or goldens.
