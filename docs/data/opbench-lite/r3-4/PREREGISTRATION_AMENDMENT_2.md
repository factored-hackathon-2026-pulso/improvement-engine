# R3-4 preregistration amendment 2 — unmapped complaint status coverage

**Status:** registered before any successful R3-4 result table was generated
or inspected. The original run failed closed on a non-empty linked complaint
status outside the two frozen semantic buckets. This amendment records that
schema limitation explicitly; it is prompted by the failed schema check and
does not make the analysis confirmatory. R3-4 remains retrospective and
descriptive.

The existing buckets remain unchanged:

- `Open`, `In Process`, and `Escalated` map to `open_like`.
- `Resolved` and `Closed` map to `resolved_or_closed`.
- Blank status and every other non-empty status map only to the existing
  `status_unknown` coverage count. Unknown values are not normalized into new
  labels, split by spelling, or included in either outcome bucket.

The runner never emits, logs, hashes, or copies an unmapped status value. The
same k=10 and complementary-suppression rules apply jointly to status outcome
and coverage tables. If their cells and complements cannot be shown safely,
both tables remain suppressed. This amendment does not change the analysis
population, joins, allowlisted columns, complaint-category mapping, query
taxonomy, E0 outcome definitions, or permitted interpretation. A missing or
unmapped status remains unknown, not open, resolved, or a measure of agent
performance.

This amendment is versioned because it follows a failed schema validation
against the supplied snapshot. Results generated under it must be described as
exploratory/descriptive and must cite this amendment; they cannot be presented
as a blind or confirmatory test.
