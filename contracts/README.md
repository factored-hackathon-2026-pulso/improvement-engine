# Pulso wire contracts

This directory contains versioned JSON contracts owned by the improvement
engine. It contains neither executable Agent Core entities nor bank data.

`artifact-envelope.schema.json` defines the Pulso v1 envelope for an immutable
revision: contract version, opaque tenant, identity/digest/kind, payload, and
lineage references. `artifact-ref.schema.json`, `source-ref.schema.json`, and
`source-snapshot.schema.json` define minimal references to Pulso revisions and
the read-only source. Payloads are specialized by `kind` in later slices; these
contracts do not replace Agent Core schemas.

In SourceSnapshot v1, `file_digest` always retains the SHA-256 of the source
object. Optional `partition_inventory_digest` commits to the distinct
partition inventory for adapters that read partitioned sources; older
snapshots without that field remain valid, while consumers requiring partition
inventory must reject them if it is absent.

References carry tenant identity so consumers can reject cross-tenant lineage
before persistence. `sources/call_center_interactions.v1.json` and
`sources/complaints.v1.json` describe the original-bank contacts and complaints
profile in source-contract version 1.0, aligned to the supplied data
dictionary. The local CLI validates that
both tables are present, every CSV has the exact ordered contract header, and
rows are structurally rectangular. A valid result does not validate individual
value types or nullability, keys or relationships, the other eleven tables,
nor produce a SourceSnapshot or digest. Its explicit validation scope is
`contacts_and_complaints_header_structure`; it does not certify the entire
bank extract or prove relationships between contacts and complaints.

The contracts do not copy or modify source CSV files. Complaint `description`
is explicitly classified `restricted` and is excluded from
`permitted_classifications`; downstream analytical projections must not consume
it. The structural validator parses rows only to check CSV shape and discards
their values. Free text, identifiers, transcripts, and other row values are
never emitted in validation summaries. Fixtures are synthetic and contain no
PII, credentials, or source rows.

Operational boundary: this validator is a local operator tool for a trusted,
operator-selected dataset directory, not an upload endpoint for untrusted
files. It streams CSV records but currently does not impose explicit limits on
file count, directory depth, or an individual record's size; do not expose it
to arbitrary user-controlled paths. Add bounded inventory/record limits before
using it in a multi-tenant service or against untrusted inputs.

Validation is available without additional dependencies:

```powershell
python -m unittest discover -s tests -v
python contracts/validate_fixtures.py
```

The second command checks that the positive fixture is accepted and every
negative fixture remains rejected. This is not a complete JSON Schema
implementation: Rust and downstream consumers must validate the published
schema in addition to these semantic invariants. RFC 8785/JCS canonicalization
and digest generation are not implemented; fixture digests exercise shape
only and do not attest canonical bytes.
