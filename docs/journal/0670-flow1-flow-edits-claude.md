# 0670 FLOW1 additive flow edits (UTC 2026-10-05, CLAUDE)

The engine proposes FLOWS only (no knowledge). Decision table and guarantees: `docs/dev/FLOW_EDITS.md`. Four additive ops in `reasoning::flow_edits`: `add_validator`, `insert_ask`, `insert_notice`, `insert_ack`; never touches rule/decide/confirm/verify/escalate/end/write nodes or edges leaving them; recompute `check_invariants` (pass-through contraction, protected nodes byte-identical, failure exits stay safe); minor flow bump, agent patch bump, exact REG-PIN follows.
Mapping: kind `flow_edit` rows appended (no rank moved); Builder (flash) picks a position and a preset from menus of the real graph; independent flow review (pro) on recomputed structured facts; suites `flow_validator|flow_ask` fail on the base natively, `flow_notice|flow_ack` cannot (no node/template id in events): proof ends non_discriminating, never announced.
Tests (TDD: Rust unit RED shown with a stub, then GREEN; Python 12 RED on old code): reasoning crate all green incl. new `tests/flow1.rs`; `scripts/regression/tests/test_flow1.py` 12 OK.
Not done / status of the live proof: see the PR description.
