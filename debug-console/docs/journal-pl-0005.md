# Journal PL-0005 (PL-L4): Sources view

- New hash route `#/sources` (nav "Fuentes"/"Sources"), `src/features/SourcesView.tsx`, demo model `sourcesModel.ts`, strings `src/i18n/sources.ts` (es-419 default + en, key parity tested).
- Everything is a `stand_in` demo model mirroring platform-exporter vocabulary (profile.py capabilities; finding/counter names unknown_event_type, denied_event_type, gap_suspected, late_event). Not read from a real API; banner says so.
- Vocabulary reconciled with the exporter (follow-up): counters are real names only (events_sent, unknown_event_type, denied_event_type, gap_suspected, late_event); no `window_revisions` counter exists (only `window_revision_required` on late_event findings). Simulator exclusion is shown as a separate model field (observations carry evidence_kind=team_generated / population_excluded), not a counter. `event_log` pseudo-capability removed; operational families on sources without an event timeline are `unsupported` with a "requires an event timeline" note. `tests/unit/exporterVocabulary.test.ts` pins counter and capability names to lists copied from the exporter.
- Reason `no_core_artifact_in_phase1` is proposed vocabulary, pending Codex; statuses use only spec 32.3 codes (waiting_dependency / insufficient_human_evidence).
- View is read-only by construction (no buttons/links). The console has no role/permission pattern yet (session has `scopes` but nothing gates on them).

## Backoffice follow-ups (not done)
- Locale switch: only this view is bilingual; the rest of the console is es-419 only.
- Nav shell is a flat link row; a sectioned backoffice nav with scope-gated entries is needed.
- Role gating from session `scopes` for decision actions; this view needs none.
- Real endpoint for source profile/counters (contract with control-api) to replace the stand_in model.
- No lint script exists in package.json (typecheck/test/build only).
