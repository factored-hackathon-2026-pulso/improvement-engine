# 0221 AG2 more cells: M7-M10 (UTC 2026-10-05T02:45Z, CLAUDE)

Lane AG2, branch `claude/ag2-more-cells`. Extends `scripts/aggregate/bank_cells.py` (stdlib; pyarrow 25.0.1 resolves offline through uv but is not needed) with four metrics from the bank dataset; definitions, supports and cannot-support in `docs/data/bank-cells-metrics.md`.

Delivered
- M7 digital Error share by action x channel (descriptive), M8 sends to non-consenting customers by campaign_type x channel (RISK), M9 transaction declines by channel x customer_segment (expected flat non-finding), M10 handled-time share on unresolved contacts by reason x channel (whole hours, re-expression of M1).
- Privacy: k = 10 on (valid, pos) and complement, no margin rows, anonymous digital events and structural-error-free actions excluded, reference files read by column allowlist (customers: id, segment, consent; campaigns: id, type), nothing row-level written.
- Rust, minimal and unavoidable: `steps::cells` now allows dims `action`, `campaign_type`, `customer_segment`, and `Config::default` lists `("M10","M1")` so M10 is flagged `depends_on` M1. Everything else in the sensor is metric-generic.

Real run (aggregates only): 7,995 cells (M7 1,960, M8 512, M9 1,447, M10 419) from 15.18M digital events, 1.72M sends, 4.30M transactions; about 10 min stdlib. `steps_cli cells`: M7, M8, M9 each reported once as refuted / no differential; M10 has 2 corroborated cells, both `depends_on` M1; `score_findings.py` on opbench-lite: recall 1.0, 0 non-findings reported.

Findings about the sensor: M7's real gap (about 6.0% vs 4.5%) is under the 5 pp effect floor, so the default engine does not call it a finding; M8's risk is a level (about 50% of sends), not a cell contrast, so a vs-rest sensor cannot surface it. Both stay descriptive evidence in the cells.

Validation: `cargo test -j 1 -p steps` (22 cells tests incl. 2 new, all steps tests green, run only when no other cargo was active), unittest scripts/aggregate (new synthetic fixtures incl. k and no-margin rules), owners test, dataclass gate push-scan.
