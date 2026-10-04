# platform-exporter

Read-only exporter of the platform `event_log` to Pulso observations (`POST /internal/v1/platform/observations`,
contract `pulso-observations-2`, catalog 1.1.0). Per-attempt service JWT with `iat`, tenant-scoped; redaction,
quarantine, gaps and late events handled client-side.

Real-ingest semantics honoured (P1): a 202 receipt with `disposition=quarantined` is not an ACK (partition stops,
cursor kept); late rows inside the fast stream are declared as holes and delivered through the late path; rows the
exporter withholds (unknown/denied type) keep sequence continuity through a payload-free type stub.

Live test against the Rust control-api (subprocess of a prebuilt binary; skips if missing, set `CONTROL_API_BIN`):

    python -m pytest tests/test_live_control_api.py -p no:cacheprovider

## Withheld rows and open gaps

Rows the exporter withholds (a bad timestamp or unparseable payload, or an unknown or denied event type) are not sent as content: the exporter declares a hole with `reason=row_withheld` and `backfill_requested=false`. The Rust control-api keeps a declared hole in `open_gaps` until a late row closes it, so a withheld-row hole can stay open indefinitely by design. That is the intended, honest signal that a row was deliberately not exported; it is not an error to be retried.
