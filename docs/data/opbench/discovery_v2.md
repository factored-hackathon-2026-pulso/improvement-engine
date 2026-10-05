# OPBENCH-lite v2 preregistration

**Status:** the metric/cell protocol was preregistered in commits
`952092a9`, `78f9fe08`, and `a994ba24` before successful v2 catalog generation;
the last commit registers the observed `Web Chat` channel alias. The current
copy adds a post-result cohort-identity caution below. That caution does not
change metric definitions or catalog selection, but the initial E0 cohort
comparison remains exploratory until membership identity is independently
verified. Sensor-vs-catalog scores are also cross-protocol agreement on the
same source snapshot, not independent accuracy. The v1 artifacts remain
historical and are not overwritten.

**Owner:** CODEX / X-DOC
**Purpose:** extend the aggregate-only benchmark to the independently audited
findings that v1 could not represent, while correcting its E0 denominator
interpretation. This is a descriptive benchmark over the supplied hackathon
snapshot and generated E0 enrichment, not a claim about real-bank prevalence,
causality, legal noncompliance, savings, or lift.

## 1. What v2 will and will not answer

The primary bank question is where first-contact unresolved rates are higher
by normalized reason across channels. The audited dataset supports an overall
Queja difference and also ranks Comercial, Técnico, and Retención as
high-unresolved reasons. It does not reveal why those contacts were unresolved.
The v2 contact sensor therefore registers all reason-by-channel cells before
results, highlights the cells requested below, and never treats a reason or
channel label as a causal mechanism.

V2 also reports three strictly separate context measures:

- the aggregate concentration of digital `Error` events on the predeclared
  transaction-action group (no contact/customer linkage);
- the observed share of marketing sends addressed to customers whose
  `accepts_marketing` flag is false (a risk indicator, not a causal or legal
  conclusion);
- the E0 `case.complaint_id` to bank `complaints.complaint_id` linkage coverage
  (a source-capability fact, not a customer/contact linkage).

The E0 recurring-query measure is corrected and retained as context, but it is
not a new opportunity: the audited query is already covered by the existing
`leer_movimientos` read tool. The audit found no real agent outlier; v2 will
not create or rank an agent-specific opportunity. It may retain this only as
an aggregate negative-control statement with no agent names, IDs, or ranks.

## 2. Inputs and data boundaries

V2 reads only these named inputs locally:

| Source | V2 use | Boundary |
| --- | --- | --- |
| `call_center_interactions` | M1 unresolved contacts, M2 complaint share, M3 complaint share among unresolved | Primary key dedupe; use `customer_id` only transiently for stable A/B assignment; never emit IDs |
| `complaints` | M4 status/open-like, M5 source `sla_breached` flag, E0 join target | `complaint_id` is used transiently only for exact E0 linkage; PQR state is a final snapshot, not event-time history |
| `satisfaction_surveys` | M6 low-CSAT negative/derived control | Exact interaction linkage only; survey `send_channel` is distinct from contact channel; exclude NPS/CES |
| `digital_events` | One aggregate error-rate contrast by the fixed action groups | Stream all source rows; no contact/customer joins, no event values or identifiers in output |
| `campaign_sends` | Consent-risk numerator and denominator | Transiently join `customer_id` to `customers.accepts_marketing`; emit aggregate only |
| `customers` | The consent flag for the campaign risk measure only | No customer profile/segment inference or identifier output |
| E0 operational `case` | E0 case denominator, complaint-ID linkage coverage, case-hash split if needed | No `labels`, `timeline`, transcripts, free text, or individual case output |
| E0 operational `copilot_query` | Determine whether a case had the frozen leading query | Query signature remains local-only; output counts/rates only |

No other input table is admitted by this preregistration. In particular, v2
does not join digital events to contacts or customers; does not join bank PQRs
to contacts, surveys, transactions, or customer histories; and does not inspect
E0 labels, timeline, transcripts, or customer identity. Source rows and join
keys stay transient in the local process. No raw or row-level artifacts are
written to Git or output directories.

## 3. Frozen normalization and metric definitions

### 3.1 Contact reason and channel

The audited contact extract has six observed reason values. Normalize the
closed set, accent-insensitively, as follows:

| Canonical reason | Accepted observed label(s) |
| --- | --- |
| `complaint` | `queja`, `reclamo`, `complaint` |
| `transactional` | `transaccional`, `transactional`, `transaction` |
| `technical` | `tecnico`, `técnico`, `technical` |
| `commercial` | `comercial`, `commercial` |
| `retention` | `retencion`, `retención`, `retention` |
| `product` | `producto`, `product` |

The first v1 limitation is explicitly corrected here: `comercial` and
`retencion` must not collapse into `unclassified` or be omitted. If an
unrecognized nonempty reason appears, fail closed with a bounded schema-drift
diagnostic; do not map it to an existing reason. Missing/empty labels are
counted as unknown coverage and excluded from reason-specific tests.

Normalize the six observed contact channels as follows:

| Canonical channel | Accepted observed label(s) |
| --- | --- |
| `phone` | `phone` |
| `email` | `email` |
| `mobile_app` | `app` |
| `whatsapp` | `whatsapp` |
| `web_chat` | `web_chat`, `Web Chat` (case-insensitive) |
| `web` | `web` |

Alias matching folds case and surrounding whitespace only; it does not use
substring or fuzzy matching. The audit records these as the six values of the
raw `call_center_interactions.channel` field. `Inbound Call` and `Outbound
Call` belong to `interaction_type`, not `channel`. Any new nonempty channel is
a schema-drift failure, not silently placed in `other`. Keep `web_chat`
separate from `web`; keep `whatsapp` separate from both. Inbound/outbound
phone direction is not a separate cell in v2.

### 3.2 Contact metrics M1–M3

- **M1 `contact_unresolved_rate`:** `was_resolved=false` / contacts with a
  parseable `was_resolved`; grain contact; cell `reason_category × channel`;
  comparator is all other normalized reasons in that same channel. This is a
  first-contact/final-extract flag, not eventual case closure.
- **M2 `complaint_share_of_contacts`:** normalized complaint contacts / all
  contacts with a known reason, by the six canonical channels plus overall.
- **M3 `complaint_unresolved_share_of_unresolved`:** unresolved complaint
  contacts / contacts with known unresolved status, by the six canonical
  channels plus overall. It is descriptive context, not an independent causal
  measure.

The eleven predeclared priority M1 cells are:

| Priority family | Cells |
| --- | --- |
| Queja across every audited channel | `complaint × phone`, `complaint × email`, `complaint × mobile_app`, `complaint × whatsapp`, `complaint × web_chat`, `complaint × web` |
| Other high-unresolved reasons | `commercial × phone`, `commercial × email`, `commercial × mobile_app`, `technical × phone`, `retention × phone` |

All 36 M1 reason/channel pairs are nevertheless enumerated and tested; the
eleven priority cells are not the only cells allowed into the catalog. This
preserves negative controls and prevents post-result selection. No channel is
claimed to be a unique cause; the audit reports the Queja excess on every
channel.

### 3.3 PQR and survey controls M4–M6

- **M4 `pqr_open_like_rate`:** status in `{Open, In Process, Escalated}` /
  PQRs with a recognized status, by the five audited PQR categories plus
  overall. This is a final status snapshot, not an aging/backlog trajectory.
- **M5 `pqr_sla_flag_rate`:** `sla_breached=true` / PQRs with a parseable flag,
  by the five audited PQR categories plus overall. The source has no independently
  verified SLA eligibility/deadline; this is a flag rate only, not certified
  SLA performance.
- **M6 `survey_low_score_rate`:** linked CSAT rows with `main_score <= 2` /
  linked CSAT rows with valid 1–5 score, by `reason_category × survey
  send_channel`, plus overall. NPS and CES are excluded. Delivery-channel
  mapping is `Email→email`, `IVR→phone`, `App→mobile_app`, `Web→web`, and
  `SMS`/unsupported delivery labels→`other`. This is a registered negative /
  dependency control: audit evidence says CSAT is primarily a function of the
  resolved flag. It must not be promoted as a separate reason-level cause or
  opportunity without independent evidence.

PQR `category` has its own exact closed map: `Transactions→transactions`,
`Fees→fees`, `Technical→technical`, `Branch→branch`, `Service→service`.
Do not pass PQR categories through the contact-reason normalizer; v1's reuse of
the generic reason map would lose `Fees` and `Service`.

### 3.4 E0 E1 correction and existing-capability disposition

E1 is the proportion of distinct E0 cases with the frozen dominant
`query_signature`, not a count of repeated queries within one case. The
signature itself is never serialized. The audited denominator is all eligible
cases in each stated partition: initial sample `154/200`, holdout/replay
`1,433/1,800`, full E0 `1,587/2,000`. Do not use the older `1,433/1,539`
denominator or the resulting 93.1% statement.

**Arithmetic reconciliation:** the task brief refers to “79.4%, 1,433/1,800,”
but that quotient is 79.611…%, which rounds to **79.6%** to one decimal. The
audited report's section 4 also labels 79.4% as the *analyst-half* statistic,
not the 1,433/1,800 holdout statistic. V2 must publish exact numerator and
denominator and derive the displayed percentage from them; it must not hardcode
79.4% for 1,433/1,800. If a different holdout denominator is later evidenced,
it requires an explicit audit correction before results are regenerated.

**Cohort identity gate:** the E0 exporter must reproduce the audited initial
200-case cohort using the same frozen membership rule, not merely a partition
of the same 2,000 cases with the same size. Timestamp ordering and a stable
case-key tie-breaker are deterministic, but are not evidence that they match
the audit's original sample selection. Documenting the membership rule and a
synthetic regression are necessary but insufficient: before claiming validated
benchmark scoring, a local real-data run must also demonstrate identity with
the audited initial cohort (for example, by comparing a private membership
fingerprint that is never committed) and reproduce the audited
`154/200`, `1,433/1,800`, and `1,587/2,000` aggregates under that same rule.
Until then, the initial-sample comparison is not independently reproducible
and claim-level recall/precision against this catalog is not validated. The
holdout/full-snapshot descriptives remain separately labeled; this gate does
not change their denominators or turn them into causal evidence.

Represent E1 as a descriptive E0 workflow-context entry with
`actionability=covered_existing_capability` and name the already-existing
`leer_movimientos` tool as coverage evidence. It is not a proposal opportunity
or a new-tool recommendation. The E0 frequency is generator-encoded and is
not bank prevalence, real query demand, proof of duplicate work, or evidence
that a proposed change improves resolution.

### 3.5 New descriptive context entries

- **Digital error concentration:** one fixed contrast only. Numerator is
  `event_type='Error'` events whose action is one of `view_transactions`,
  `initiate_transfer`, `initiate_payment`; denominator is all events for those
  three actions. Compare descriptively with the predeclared other-view group
  `{view_help, view_home, view_accounts, view_products}`. Do not include null
  action, authentication actions, or unregistered actions in either group.
  Publish the two group rates, counts, absolute difference, and audited source
  coverage. No p-value, causality, customer/contact linkage, or predicted
  contact reduction. It is `descriptive_only`, not `problem`.
- **Marketing consent risk:** numerator is sends whose linked customer has
  `accepts_marketing=false`; denominator is sends with a resolved, valid
  consent flag. Publish overall and the five fixed send-channel strata
  `{Email, Push, SMS, Voice, WhatsApp}` only when disclosure floors hold. It is
  `type=risk`, not a customer-service problem. The audit found the share
  matches the roughly 50% customer base rate and does not alter open/conversion
  rates; this is not proof of a causal marketing effect, legal violation, or
  current non-consent policy breach. Do not propose marketing automation from
  this metric alone.
- **E0-to-bank linkage coverage:** exact join `case.complaint_id` to
  `complaints.complaint_id`; numerator resolved cases, denominator eligible
  E0 cases. Audit coverage is 2,000/2,000. Output only aggregate counts/rate,
  no IDs, per-subcategory breakdown, customer join, or row-level evidence.
  Label as `context_only`; E0 enrichment was generated and cannot be read as
  bank-wide prevalence.

These three entries have no inferential discovery decision. Their schema must
record `multiple_testing.method=not_applicable_preregistered_descriptive`,
`adjusted_q=null`, and a reason that no hypothesis test was run. Do not invent a
p-value or label them significant.

## 4. Discovery, holdout, and multiplicity

### 4.1 Fixed test family

All v2 hypothesis tests form one global Benjamini–Hochberg family at
`q <= 0.05`. The registered cell inventory is:

| Metric | Planned cells included in family |
| --- | ---: |
| M1 | 6 reasons × 6 contact channels + overall = 37 |
| M2 | 6 contact channels + overall = 7 |
| M3 | 6 contact channels + overall = 7 |
| M4 | 5 PQR categories + overall = 6 |
| M5 | 5 PQR categories + overall = 6 |
| M6 | 6 reasons × 5 survey delivery groups + overall = 31 |
| E1 | One frozen initial-sample vs holdout comparison = 1 |
| **Total** | **95** |

Overall reference cells, unsupported cells, and cells without a usable p-value
remain in the catalog and reserve `p=1` slots as appropriate. Every inferential
entry carries the exact metric family, `cells_explored.family_size=95`,
`multiple_testing.method=two_proportion_z_benjamini_hochberg_fdr`, and adjusted
q-value or explicit null/suppression reason. The three descriptive context
entries above use the registered not-applicable marker instead.

### 4.2 M1/M2/M3/M4/M5/M6 test rules

- Split bank rows by the same deterministic customer-hash rule as v1; the same
  customer remains in one half across eligible bank tables. Keys and hashes
  stay local-only. Discovery is split A; B is the frozen replication half.
- For M1/M2/M3/M4/M5/M6 cells, compute the two-proportion discovery comparison
  against the registered complement (same channel for contact metrics; same
  category/metric complement where specified). Use the existing v1
  two-proportion test and Wald interval unless a registered cell's structure
  makes that comparison undefined, in which case mark it non-testable and
  retain a family slot at `p=1`.
- Privacy floor `k=10`: never release a count below 10; for a binary measure,
  suppress numerator, denominator, rate, interval, and effect if either
  positive or negative support is 1–9. Additionally, an M1 focal cell and its
  same-channel complement must each have at least 500 known records over the
  full snapshot for an inferential finding. Each split must meet `k=10` in
  both events and non-events. If not, status is `uncertain/underpowered`, not
  zero and not refuted.
- M1 is an adverse unresolved-rate scan. A discovery candidate requires a
  positive difference of at least 5 percentage points, rate ratio at least
  1.25, supported denominators, and q<=0.05 in A. Freeze only A-selected
  candidates before looking at B. Replication requires the same adverse
  direction, at least the same 5 pp / 1.25 ratio floors, and q<=0.05 after BH
  over the frozen candidate set. Lower unresolved rates are not adverse
  findings even if statistically different.
- M2/M3/M4/M5/M6 preserve the v1 5 pp practical-difference floor and status
  vocabulary, but M6 is a dependency/negative control and cannot generate a
  standalone proposal. A result contradicting an audited flat/dependent
  measure is published as a discrepancy, not suppressed or tuned away.
- Secondary M1 persistence check: among the 35 full literal-naive months
  2023-07 through 2026-05, require the same adverse sign in at least 80% of
  months with at least 100 eligible rows. It is robustness context, not a new
  hypothesis family or an onset/drift claim. Do not order events across tables
  below day resolution or construct as-of outcomes from final-extract fields.

### 4.3 E1 and descriptive measures

E1's registered inferential comparison is initial 200-case sample versus the
1,800-case holdout, using distinct cases and the corrected denominator. It
shares the 95-cell global family but is reported only as E0 synthetic workflow
context and `covered_existing_capability`. The existing `154/200` initial and
`1,433/1,800` holdout figures are not independent bank samples and do not
establish temporal stability or expected lift.

The digital error contrast, marketing consent risk, and linkage coverage are
predeclared aggregate descriptions, not tests. V2 does not calculate a p-value
or add these rows to the discovery family. If the implementation elects to
introduce inferential channel/month/action tests later, it requires a new
preregistration version before any such results are computed.

## 5. Catalog, output and versioning contract

- Preserve the current `results/opbench-lite.json` and `results/cell_audit.json`
  byte-for-byte as the v1 record. V2 outputs must be written only to
  `results/v2/opbench-lite.json` and `results/v2/cell_audit.json` after this
  preregistration is committed.
- Add a strict v2 JSON schema with `version="2"`; retain the v1 schema and
  do not make one schema silently accept both versions.
- The v2 catalog contains every registered inferential cell (95 rows,
  including overall/unsupported/negative-control rows) plus exactly the three
  descriptive context entries (digital action contrast, marketing consent
  risk, E0 complaint-ID join). A top-level `negative_controls` object
  references the four inferential controls M2-phone, M3-phone, M4-technical,
  and M5-technical and records the aggregate audited no-agent-outlier finding
  without identifiers or ranks. It is not an entry, adds no test-family or
  descriptive-entry slot, and is explicitly audit-derived (not recomputed
  unless a separately validated source supports it). All entries include a
  stable metric/cell key,
  definition, disclosure-safe aggregates, `cells_explored`, multiplicity
  disposition, evidence status, `type`, `actionability`, and caveats.
- Use explicit evidence status separate from disposition. E.g. E1 may be
  `corroborated_descriptive` as recurring E0 behavior while
  `actionability=covered_existing_capability`; this prevents statistical
  recurrence from being presented as an open opportunity. Context rows use
  `actionability=context_only`; marketing uses `type=risk` and never
  `problem`.
- Preserve at least five audited negative controls in the published catalog
  when their source evidence remains consistent: M2 phone complaint share,
  M3 phone complaint share among unresolved, M4 technical-category open-like
  rate, M5 technical-category SLA-flag rate, and the aggregate
  no-agent-outlier result. The four metric controls are ordinary rows in the
  fixed 95-cell family; the no-agent-outlier result is represented in the
  top-level `negative_controls` object because it is audit-derived, not a
  recomputed cell. If recomputation contradicts a metric control, publish the
  discrepancy rather than forcing its previous `refuted` status.
- The complete audit catalog and schema validation must reject source IDs,
  signatures, raw text, paths, customer-level records, and counts/rates that
  violate `k=10`. Derived percentages are calculated from exact stored counts;
  do not hand-enter rounded rates.

## 6. Preregistered interpretation and non-findings

The v2 headline, if reproduced, is: “Queja and selected other reasons have
higher first-contact unresolved shares in this supplied snapshot; the
aggregate identifies where, not why.” It is not “the channel causes failure,”
“the PQR caused the contact,” or “a new agent will reduce unresolved contacts.”

Specific non-findings retained as controls:

1. Agent-level resolution/escalation/follow-up outliers: the audit reported
   dispersion ratios 0.97/0.99/0.96 and no outlier evidence across 1,090
   agents. No individual agent identifiers or rank list are emitted; do not
   create an agent-focused proposal.
2. PQR SLA-flag rates remain a supplied flag, not validated deadline
   performance; audit reported no material category/channel differentiation.
3. PQR open-like status remains a final snapshot and does not establish
   backlog aging.
4. Low CSAT is dependent on resolved status and is not NPS or an independent
   reason-specific cause.
5. E0 query recurrence is already covered by `leer_movimientos`; it does not
   justify a new tool proposal.
6. Consent risk is not a discovered increase versus the customer base rate,
   nor a causal effect on conversion. Keep it labeled risk/context only.

The aggregate agent-outlier statement is retained in the top-level
`negative_controls` object, not emitted as an additional benchmark entry. It
records only the audit's aggregate non-finding and is not represented as a
source-recomputed metric.

Non-finding states require adequate support. Missing, suppressed, or
underpowered evidence must be `uncertain`, never `refuted`. Do not select or
remove negative controls after viewing v2 outputs.

## 7. Known evidence and implementation gaps before generation

The audited source file in the workspace is
`docs/reports-claude/BANK_DATA_AUDIT_2026-10-04.md`; it is not currently in
this engine checkout. Its verified aggregate claims that motivate this
preregistration include:

- Queja 66,000/117,021 = 56.4%, versus other reasons 94,266/569,275 = 16.6%,
  +39.8 pp; same-direction evidence across customer halves, all audited
  channels, years, and 37/37 months.
- Retención 8,198/20,578 = 39.84%, Comercial 19,093/54,879 = 34.79%, and
  Técnico 30,940/102,899 = 30.07%; no separate causal mechanism is identified.
- Digital transactional-action group 5.97% (2,705,088 events) versus other
  views 4.53% (3,565,262); customer contact within seven days was not higher
  after sampled-day errors, so no contact linkage/opportunity is claimed.
- Sends to non-consenting customers 874,417/1,746,801 = 50.06%, at the base
  rate; no causal conversion/open effect.
- E0 `complaint_id` maps 2,000/2,000 cases to bank PQR rows; this is the only
  bank join available to E0 and is generated/enriched data.
- E0 leading-query holdout is 1,433/1,800 (79.6% by direct arithmetic), not
  1,433/1,539 (93.1%); `leer_movimientos` already covers that lookup.
- Agent-outlier analysis is flat; do not promote an individual agent.

Before generating output, the implementation must close these gaps without
changing this preregistration:

1. Add explicit `commercial` and `retention` contact-reason aliases and keep
   the six audited channel groups distinct (`phone`, `email`, `mobile_app`,
   `whatsapp`, `web_chat`, `web`); accept only the raw values listed in §3.1.
2. Add a separate PQR category normalizer for the exact five values; do not
   pass fees/service/transactions through the contact reason vocabulary.
3. Extend the stream-safe aggregate adapter to read `digital_events` without
   contact joins and `campaign_sends` with only the consent boolean from
   `customers`.
4. Extend the E0 aggregate export to compute case-level dominant-query
   denominators for 200/1,800 cases and exact complaint-ID linkage counts,
   without outputting IDs or signatures.
5. Add v2-only schema, catalog, and regression fixtures; test corrections to
   OPB-04, the six-channel/six-reason complete family, the digital grouping,
   the consent risk label, E0 linkage, agent no-outlier nonfinding, and all
   privacy floors before running against source data.

This is the freeze point. Any changed metric, comparison, threshold,
normalization, channel group, test family, or desired output after result
generation requires `discovery_v3.md` committed before recomputation.
