import { expect, it } from 'vitest';
import { CAPABILITIES, DEMO_SOURCES } from '../../src/features/sourcesModel';

// Copied (read-only) from platform-exporter: service.py PollReport + `bump[...]` counters; profile.py CAPABILITIES.
const EXPORTER_COUNTERS = ['batches_sent', 'events_sent', 'duplicate_acks', 'deferred', 'quality_findings', 'late_events', 'bad_row',
  'denied_event_type', 'unknown_event_type', 'late_event', 'gap_suspected'];
const EXPORTER_CAPABILITIES = ['origin', 'topic', 'complaint_id', 'identity_check', 'routing_step', 'copilot_query', 'tool_call', 'approval',
  'suggestion', 'case_close.resolved', 'csat', 'teams'];

it('demo counter keys are a subset of the exporter vocabulary', () => {
  for (const s of DEMO_SOURCES) for (const k of Object.keys(s.counters ?? {})) expect(EXPORTER_COUNTERS, k).toContain(k);
});
it('demo capabilities are a subset of the exporter capability manifest', () => {
  for (const c of CAPABILITIES) expect(EXPORTER_CAPABILITIES, c).toContain(c);
});
