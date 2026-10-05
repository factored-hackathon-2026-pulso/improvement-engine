# Proposal acceptance check

This is a read-only, Python-standard-library HTTP checker for a proposal in
the local Agent Core registry. It does not import the engine, inspect a
database, create proposals, or call lifecycle mutation routes.

```powershell
python scripts/acceptance/check_proposal.py `
  --proposal-url http://127.0.0.1:8001/v1/registry/proposals/<proposal-id> `
  --history-url <approved-read-only-history-endpoint> `
  --quota-url <approved-read-only-quota-observation-endpoint>
```

The proposal URL matches the pinned read-only registry detail route
(`GET /v1/registry/proposals/{proposal_id}`); the checker verifies its current
state is `draft`; the returned `proposal_id` must match the ID in the requested
URL. This does not establish who performed prior lifecycle actions. URLs must
be loopback HTTP(S), and redirects are rejected so a local
endpoint cannot forward the checker (or its bearer token) to another host.
The pinned registry OpenAPI has no read-only actor-history endpoint
and no read-only quota endpoint. Those checks are optional and, without an
approved observation endpoint, are reported as `not_exercised` with exit status
2; they are never reported as passes. A connection/HTTP/JSON failure prints a
bounded error and also exits 2. Set `PULSO_ACCEPTANCE_TOKEN` in the process
environment only when the local endpoint requires a bearer token; the token
is never printed.

The optional read-only observations accept these **unverified adapter shapes**:

- History: `{"actions":[{"action":"draft_created","actor":"engine"}]}`.
  An engine-attributed approve/publish/promote action fails the check.
- Quota: `{"used_24h":1,"limit":10,"within_limit_accepted":true,"over_limit_rejected":true}`.

The pinned Agent Core API does not define either shape as an authenticated,
proposal-bound, complete observation contract. Therefore supplying a
syntactically valid object never marks lifecycle-history or quota enforcement
as passed: those checks remain `not_exercised` and the process exits 2 unless a
separate assertion fails. Malformed or explicitly violating observations may
fail the check, but a clean-looking fixture is not proof that the engine never
mutated lifecycle state or that a 24-hour tenant quota was enforced. In
particular, `GET /proposals/{pid}` and `GET /releases/{rid}` expose current
resource detail; absent documented binding/completeness semantics they are not
actor-attributed immutable history or quota evidence. The quota value in
`docs/dev/LOCAL_STACK.md` is a configured limit, not proof of runtime
enforcement. Do not fabricate an endpoint or turn a recorded-response fixture
into a live pass.

Recorded HTTP-response fixtures under `fixtures/` are synthetic and exercise
the response contract only. They are not evidence that a live proposal exists,
that Agent Core enforces quota, or that an engine action was never made. Run
the focused tests with:

```powershell
python -m unittest scripts.acceptance.tests.test_check_proposal -v
```

The checker accepts the pinned `ProposalDetail` envelope shape: `proposal`,
`changes`, and `last_eval` are required; `review` is optional. For this
draft-only acceptance path, `last_eval` and `review` must be null. It validates
the `Proposal` summary's required fields, allowed fields, types, enums, title
length, agent-id pattern, and timestamp format, but is not a general JSON
Schema validator. It validates each change against the generic pinned
`EntityDraft` shape: nonblank bounded
`kind`, nonempty JSON-object `content`, and the required bounded `docs`
fields. It does not validate the content against the artifact-kind-specific
schema or prove that the proposed delta is semantically useful. It scans the
complete proposal plus artifact changes for obvious PII canaries. Each change
must contain all seven dossier headings with nonempty section bodies; the
evidence section must include at least two numeric values, an explicit
comparison, and a snapshot/source marker. This is a mechanical completeness
gate, not semantic validation of the claim. Unit tests cover the envelope and
generic change shape, PII inside sibling `changes[].content`, render-ready
plain-text descriptions, empty sections, and evidence without a
comparator/baseline.
