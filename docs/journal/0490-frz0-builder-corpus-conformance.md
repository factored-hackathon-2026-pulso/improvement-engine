# 0490 — FRZ0 builder-output corpus conformance

## Scope

Add a pure evaluator for the frozen FRZ0 builder-output corpus and prove that
the Rust result matches its ten published fixture verdicts. This is an offline
contract-validation helper; it does not construct or compile an Agent Core
artifact and does not grant evaluation, approval, publication, or runtime
authority.

The evaluator remains separate from `UntrustedDesignIntent`, whose DTO has a
different contract. It checks the fixture output shape and identifier syntax,
resolves evidence refs and an optional target ref against the supplied frozen
catalogue for `linked` and `do_nothing`, and returns `unlinked` / `not_evaluable`
without consulting the catalogue. If a `linked` or `do_nothing` output cannot
be checked because the catalogue is absent or malformed, it returns
`not_evaluable`; an absent ref in a valid supplied catalogue is `invalid`.
`finding_ref` is not catalogue-resolved because FRZ0 defines no finding-ref
catalogue domain. `target_ref` remains optional as specified by the schema.

## Frozen-pack ambiguity

`verdicts.json` publishes a rule requiring reference resolution for `valid`,
golden verdicts, and a `needs_catalogue` map. The map marks linked outputs
bo-01/02 and do-nothing bo-03 as not needing the catalogue even though their
refs must resolve under the published rule. Its semantics are unspecified and
the generated values are inconsistent with the rule. The evaluator does not
branch on that map. It uses the explicit rule and exact golden verdicts with
the full catalogue supplied. Tests characterize that bo-01 and bo-03 are
`not_evaluable` without a catalogue, because their references cannot be proven
to resolve. Do not change the frozen pack or generator in this slice; the pack
owner should clarify the metadata in a separately versioned contract update.

## TDD and validation

- RED: the initial corpus-conformance integration test failed to compile on
  the missing evaluator API before the implementation was added.
- GREEN: the focused Rust target passed 11/11, covering all ten golden
  verdicts, fixture filename/payload ID parity, catalogue-free
  `unlinked`/`not_evaluable` dispositions, catalogue-free linked/no-op
  `not_evaluable`, optional target behavior, independently invented evidence
  and target refs, and malformed output/catalogue boundaries.
- `cargo +1.98.1 fmt --all -- --check` and `git diff --check` passed.
- Independent adversarial review resolved the catalogue ambiguity by making
  the map explicitly uninterpreted by this evaluator and testing the
  conservative no-catalogue result. A terminology correction aligns the text
  with the plan: this is an FRZ0 builder corpus, not C-8.

## Remaining verification boundary

This entry records focused tests only. Full local CI must be rerun on the
consolidated tree before publishing these changes. No GitHub Actions result,
PostgreSQL execution, live Agent Core behavior, or business-impact claim is
asserted here.
