import { describe, expect, it } from 'vitest';
import { CONSUMER_PROPOSALS, proposalMeta } from '../../src/api/schemas';
import * as S from '../../src/api/schemas';

const schemaByName = (name: string) => (S as unknown as Record<string, Parameters<typeof proposalMeta>[0]>)[name]!;

describe('consumer_proposal registry', () => {
  it('marks every provisional schema with metadata.consumer_proposal=true and a tracking ref', () => {
    expect(Object.keys(CONSUMER_PROPOSALS).sort()).toEqual(
      ['Decision', 'Diff', 'Gates', 'Investigation', 'Memory', 'Profile', 'Session', 'StepUp'].sort(),
    );
    for (const [name, ref] of Object.entries(CONSUMER_PROPOSALS)) {
      expect(proposalMeta(schemaByName(name)), name).toEqual({ consumer_proposal: true, ref });
      expect(ref).toMatch(/^(CLQ-\d+|CO-\d+|R\d|M\d)/);
    }
  });
  it('does not mark the schemas defined by plan 16.10', () => {
    for (const name of ['Graph', 'RunList', 'DebugEventSchema', 'Problem', 'Accepted']) {
      expect(proposalMeta(schemaByName(name)), name).toBeNull();
    }
  });
});
