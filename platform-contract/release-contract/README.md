# release-contract (PX0)

How the product platform (Phase 1 model: cases, turns, assignments, event_log with `sequence`) would
consume a published agent release:

1. Resolve the alias through the registry read `GET /v1/registry/aliases/{agent_id}/{alias}`
   (`alias_resolution.schema.json`, the real wire shape of the Core registry).
2. Expose the move as an `event_log` row `release.published` (or `release.rolled_back`) carrying identity only
   (`release_event.schema.json`): `release_id`, `agent_id`, `alias`, optional `previous_release_id`.
3. The engine observes the row as an ordinary `platform_event` with `source_sequence = event_log.sequence`.

## Honest labelling

The platform side is **simulated (product-consumer)**: the real product has not published release exposure.
This stays so until EXT-2 (product answer on `event_log.payload` shapes and release exposure) is received.
The registry side is the real Core wire shape (mock verified by parity tests).

## Event catalog 1.1.0 is unchanged

`release.published` / `release.rolled_back` are NOT in `event-catalog.json` (still 1.1.0). Codex's Rust digests
that catalog, so under 1.1.0 these types classify as `unknown`: counted, quarantined with a quality finding, and
the batch still confirms (spec 32.2.5). They become `admitted` only when Codex admits them (P2R) in a later
catalog version; nothing here pre-empts that decision.

## Files

- `alias_resolution.schema.json`, `release_event.schema.json`
- `examples/valid/*.json` golden vectors, `examples/invalid/*.json` must be rejected
  (effect field in payload, missing `release_id`, wrong type, missing alias `status`)
- Python twin: `platform_contract/release_events.py` (a test keeps both in step)
- Simulator: `PlatformLiveSim.consume_release` in `platform-sim/platform_live/simulator.py`
