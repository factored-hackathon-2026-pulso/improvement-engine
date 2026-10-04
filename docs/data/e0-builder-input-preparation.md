# E0 builder-input preparation boundary

The local E0 runner emits a typed `e0_builder_input_preparation_v1` record
after assembling the measured signal candidates. This stage is the explicit
handoff boundary between a source-bound finding and a future builder-design
request; it does not invoke a model/provider, create a proposal, or materialize
an Agent Core Agent, Flow, Jev, Tool, Skill, or other artifact.

For candidates, the runner checks the exact run ID, source snapshot ID/revision/
digest, tenant-to-snapshot binding, and observation cutoff across the genuine
`LocalRunResult` and its independently validated `LocalProposalAssembly`. It
persists only candidate metric IDs and signal/summary digests as provenance.
The assembly input must come directly from `assemble_e0_proposals(run)`, which
validates candidate membership and evidence digests; the preparation boundary
rechecks run, snapshot, tenant, and cutoff identity rather than independently
reconstructing every signal commitment.
The result is `evidence_binding=bound`, while `status=dependency_blocked`
because this local simulation composition has neither a trusted U20 evaluation
plan nor the U20-E E0 safety oracle. These states are intentionally separate:
bound evidence is not builder readiness. No public status can claim `ready`.

Runs with no qualifying candidate emit `status=not_applicable` and an explicit
reason, not a negative builder finding. OriginalBank runs do not emit this E0
stage. The status and one allowlisted timeline event are written atomically
with `result.json` and `events.ndjson`; event details contain no tenant,
source-row, candidate digest, model output, or secret values.

The payload has no route/Flow reference, model prompt/output, authority grant,
U20/E0 safety commitment, executable flag set to true, or Core write. It is
not an Agent Core proposal and provides no evidence of provider execution,
evaluation, release, causal improvement, or business lift. A future trusted
composition may permit a design-only request only after real typed U20 and
U20-E inputs are supplied and rebound to the exact run/candidate; provider
invocation and subsequent Agent Core authoring remain separate steps.

## Checks

The owner records exact RED/GREEN commands, focused suites, formatting,
Clippy, and adversarial review status in the feature journal and shared team
log. Synthetic fixture tests do not substitute for an actual-source E0 smoke.
